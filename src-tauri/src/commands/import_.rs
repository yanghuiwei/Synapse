//! 导入命令：**预览差异 → 按冲突模式落库**。
//!
//! ## 为什么预览和落库读同一份计划
//!
//! 两边各算一遍「这个名字和谁冲突」必然会分叉 —— 典型症状是
//! 预览说「+3 新增」，导入完却多出 4 条，而用户没有任何办法看出哪一步错了。
//! 所以 [`build_plan`] 产出 [`Plan`]，[`to_preview`] 与 [`apply_plan`] 都只读它，
//! 谁也不许自己去查第二遍「库里现在有什么」。
//!
//! ## 三种冲突模式
//!
//! | 模式 | 同名（库里已有 + 文件里也有） | 文件里没提到的库内条目 |
//! | --- | --- | --- |
//! | `merge` 合并 | 就地更新（保留 id） | 保留 |
//! | `duplicate` 新建副本 | 新建一条，名字加序号后缀 | 保留 |
//! | `overwrite` 覆盖 | 就地更新（保留 id） | **移进回收站** |
//!
//! 「就地更新」而不是「删掉再建」是刻意的：历史记录、Token 源引用、
//! 上一次响应的快照都挂在这一行的 id 上，换个 id 等于把它们一起弄丢。
//! 覆盖模式的删除走**软删除**，进了回收站还能捞回来。
//!
//! ## 谁负责起名字
//!
//! 解析器只从文件里如实取名字，取不到就留空；**占位文案由前端随
//! [`ImportSource::labels`] 送进来**。因为那些名字会写进库、显示在侧栏上，
//! 属于界面文案 —— 后端硬编码一份中文，以后加英文界面就会露馅。

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::collection::ensure_usable_folder;
use crate::commands::validate::MAX_NAME_LEN;
use crate::error::AppResult;
use crate::importer::{
    self, EnvVarDraft, FolderNode, Format, ImportInput, ImportWarning, RequestDraft, WarningKind,
};
use crate::storage::models::{ApiRequest, AuthConfig, BodyType, EnvVar, Folder};
use crate::storage::{repo, with_conn};
use crate::AppState;

// ============================================================
// 入参与出参
// ============================================================

/// 导入时给「文件里没名字」的东西用的占位名。见模块头「谁负责起名字」。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLabels {
    pub untitled_folder: String,
    pub untitled_request: String,
    /// 集合连名字都没写时，环境只好叫这个
    pub untitled_collection: String,
}

/// 一份待导入的内容 + 它要落到哪儿。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    /// 粘贴的文本，或前端读进来的文件内容
    pub text: String,
    pub workspace_id: i64,
    /// 落点；`None` = 工作区根
    #[serde(default)]
    pub target_folder_id: Option<i64>,
    /// 只有提示价值（识别失败时告诉用户「你选的是这个文件」）
    #[serde(default)]
    pub file_name: Option<String>,
    pub labels: ImportLabels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Conflict {
    /// 同名就地更新，库里其他的一律留着
    Merge,
    /// 同名新建一条副本（名字加序号），现有条目一根汗毛不动
    Duplicate,
    /// 目标范围内以这份文件为准：同名就地更新，文件里没有的进回收站
    Overwrite,
}

/// 一个节点在预览树里的处境。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffStatus {
    /// 库里没有，要新建
    New,
    /// 库里已有且内容不同，要改写（id 不变）
    Update,
    /// 库里已有且一模一样，一个字节都不用写
    Skip,
    /// 复用目标位置上已有的分类，不新建
    Keep,
    /// 覆盖模式：文件里没提它，要进回收站
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewKind {
    Folder,
    Request,
}

/// 四个计数。界面上的 `+12 新增 / ~3 更新 / =5 跳过 / -2 移除` 就是它。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCounts {
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewNode {
    pub name: String,
    pub kind: PreviewKind,
    pub status: DiffStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<PreviewNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvPreview {
    pub name: String,
    pub var_count: usize,
    /// 同名环境已经存在 → 变量并进去，不会再多个环境出来
    pub merged: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub format: Format,
    pub counts: ImportCounts,
    pub nodes: Vec<PreviewNode>,
    pub environments: Vec<EnvPreview>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub format: Format,
    pub counts: ImportCounts,
    /// 真正写进去的环境数（`include_environments = false` 时是 0）
    pub environments: usize,
    pub warnings: Vec<ImportWarning>,
}

// ============================================================
// 命令
// ============================================================

/// 只看不写：解析 + 比对，回一棵差异树。
///
/// **没有 `include_environments` 参数**：预览里永远列出文件里带的环境，
/// 用户才知道「原来还有一批变量可以一起导」。真写不写由提交那一步的开关决定，
/// 而不是让预览瞒着不说。
#[tauri::command]
pub async fn preview_import(
    state: State<'_, AppState>,
    source: ImportSource,
    conflict: Conflict,
) -> AppResult<ImportPreview> {
    with_conn(&state.pool, move |conn| {
        let plan = build_plan(conn, &source, conflict, true)?;
        Ok(to_preview(&plan))
    })
    .await
}

/// 落库。**会重新解析一遍**（预览到落库之间用户可能换了文件），
/// 但走的是同一套 [`build_plan`]，所以名字与冲突判定和预览完全一致。
#[tauri::command]
pub async fn commit_import(
    state: State<'_, AppState>,
    source: ImportSource,
    conflict: Conflict,
    include_environments: bool,
) -> AppResult<ImportOutcome> {
    with_conn(&state.pool, move |conn| {
        let plan = build_plan(conn, &source, conflict, include_environments)?;
        apply_plan(conn, &plan)
    })
    .await
}

/// 解析 + 落库，一步到位，**只吃一个连接**。
///
/// 存在的理由：`commit_import` 是 Tauri 命令（要 `State<'_, AppState>`），
/// 非 Tauri 进程造不出来；而它内部真正干活的两步（[`build_plan`] / [`apply_plan`]）
/// 的中间产物 `Plan` 是私有类型。把这一层暴露给 MCP 侧，是为了让
/// 「AI 粘一段 curl」与「用户在界面里点导入」**走完全同一条落库路径** ——
/// 各自拼一遍落库逻辑的话，冲突判定、名字去重、回收站行为迟早分叉。
pub fn import_into_folder(
    conn: &mut Connection,
    source: &ImportSource,
    conflict: Conflict,
    include_environments: bool,
) -> AppResult<ImportOutcome> {
    let plan = build_plan(conn, source, conflict, include_environments)?;
    apply_plan(conn, &plan)
}

// ============================================================
// 计划
// ============================================================

/// 落到库里的一条条指令。预览与落库共用同一份。
#[derive(Debug)]
enum Action {
    /// 新建
    Create,
    /// 落到已经存在的这一条上（分类 = 复用；接口 = 就地更新）
    Existing(i64),
    /// 内容与库里完全一致 —— 一个字节都不用写
    Skip,
    /// 覆盖模式：文件里没有它，进回收站
    Remove(i64),
}

impl Action {
    fn folder_status(&self) -> DiffStatus {
        match self {
            Action::Create => DiffStatus::New,
            // 复用现有分类既不是新增也不是更新，界面上给个中性标记
            Action::Existing(_) | Action::Skip => DiffStatus::Keep,
            Action::Remove(_) => DiffStatus::Remove,
        }
    }

    fn request_status(&self) -> DiffStatus {
        match self {
            Action::Create => DiffStatus::New,
            Action::Existing(_) => DiffStatus::Update,
            Action::Skip => DiffStatus::Skip,
            Action::Remove(_) => DiffStatus::Remove,
        }
    }
}

#[derive(Debug)]
struct PlannedFolder {
    name: String,
    /// 要写回的授权配置。`None` = 不用动（导入没带，或者与库里已经一致）
    auth_to_write: Option<AuthConfig>,
    action: Action,
    children: Vec<PlannedNode>,
}

#[derive(Debug)]
enum PlannedNode {
    Folder(PlannedFolder),
    /// 草稿装箱：它比 `PlannedFolder` 大一圈，直接内联会让**每个**节点
    /// 都按最大变体的尺寸占地方 —— 一棵几百节点的导入树不值这个开销。
    Request {
        draft: Box<RequestDraft>,
        action: Action,
    },
}

impl PlannedNode {
    fn preview(&self) -> PreviewNode {
        match self {
            PlannedNode::Folder(folder) => PreviewNode {
                name: folder.name.clone(),
                kind: PreviewKind::Folder,
                status: folder.action.folder_status(),
                method: None,
                url: None,
                children: folder.children.iter().map(PlannedNode::preview).collect(),
            },
            PlannedNode::Request { draft, action } => PreviewNode {
                name: draft.name.clone(),
                kind: PreviewKind::Request,
                status: action.request_status(),
                method: Some(draft.method.clone()),
                url: Some(draft.url.clone()),
                children: Vec::new(),
            },
        }
    }
}

/// 已经定好名字、算好差异的环境。
#[derive(Debug)]
struct PlannedEnv {
    name: String,
    vars: Vec<EnvVarDraft>,
    merged: bool,
}

#[derive(Debug)]
struct Plan {
    format: Format,
    workspace_id: i64,
    /// 落点。**落库时从它开始向下铺**，不能默认从根开始 ——
    /// 顶层节点（不归任何分类的接口、顶层分类）都要挂在这里。
    target_folder_id: Option<i64>,
    nodes: Vec<PlannedNode>,
    environments: Vec<PlannedEnv>,
    warnings: Vec<ImportWarning>,
    counts: ImportCounts,
}

// ============================================================
// 库里现在有什么
// ============================================================

/// 目标工作区的现状快照。**一次读全**，而不是每规划一个节点查一次库 ——
/// 一份 200 条的集合会因此从几百次查询降到 3 次。
struct Existing {
    /// (父级, 名字) → 分类
    folders: HashMap<(Option<i64>, String), Folder>,
    /// (父级, 名字) → 接口
    requests: HashMap<(Option<i64>, String), ApiRequest>,
    /// 父级 → 子分类（覆盖模式要按层级走一遍）
    folders_in: HashMap<Option<i64>, Vec<Folder>>,
    /// 父级 → 接口（覆盖模式同上）
    requests_in: HashMap<Option<i64>, Vec<ApiRequest>>,
    env_names: HashSet<String>,
}

impl Existing {
    fn load(conn: &Connection, workspace_id: i64) -> AppResult<Self> {
        let mut out = Self {
            folders: HashMap::new(),
            requests: HashMap::new(),
            folders_in: HashMap::new(),
            requests_in: HashMap::new(),
            env_names: HashSet::new(),
        };

        // 同名多条时保留先出现的那条（仓储按 sort_order 排序，顺序是稳的）
        for folder in repo::folder::list_by_workspace(conn, workspace_id)? {
            out.folders_in
                .entry(folder.parent_id)
                .or_default()
                .push(folder.clone());
            out.folders
                .entry((folder.parent_id, folder.name.clone()))
                .or_insert(folder);
        }
        for request in repo::request::list_by_workspace(conn, workspace_id)? {
            out.requests_in
                .entry(request.folder_id)
                .or_default()
                .push(request.clone());
            out.requests
                .entry((request.folder_id, request.name.clone()))
                .or_insert(request);
        }
        for env in repo::env::list_by_workspace(conn, workspace_id)? {
            out.env_names.insert(env.name);
        }
        Ok(out)
    }

    fn folder(&self, parent: Option<i64>, name: &str) -> Option<&Folder> {
        self.folders.get(&(parent, name.to_string()))
    }

    fn request(&self, parent: Option<i64>, name: &str) -> Option<&ApiRequest> {
        self.requests.get(&(parent, name.to_string()))
    }
}

// ============================================================
// 规划
// ============================================================

struct Ctx<'a> {
    existing: &'a Existing,
    conflict: Conflict,
    labels: &'a ImportLabels,
    /// 已经落位的现有条目 —— 覆盖模式靠它算「哪些没被文件提到」
    used_folders: HashSet<i64>,
    used_requests: HashSet<i64>,
    /// 本次计划占用的名字（父级 → 名字）。既算库里的也算本次前面的，
    /// 否则一份文件里两条同名的会撞成一条。
    claimed: HashMap<Option<i64>, HashSet<String>>,
    /// 待建分类的负数槽位发放器
    next_slot: i64,
}

impl Ctx<'_> {
    /// 给一个待建分类发槽位。
    ///
    /// 规划的时候还没有行 id，但子节点必须知道「自己该挂在谁下面」才能查同名。
    /// 于是给待建分类发一个**负数**：它在库里注定查不到，
    /// 正好表达「这个分类还不存在，里面不可能有东西」。
    /// 少了这一手，嵌套分类会跑到错误的父级下去找同名。
    fn new_slot(&mut self) -> i64 {
        let slot = self.next_slot;
        self.next_slot -= 1;
        slot
    }

    fn taken(&self, parent: Option<i64>, name: &str) -> bool {
        self.existing.request(parent, name).is_some()
            || self
                .claimed
                .get(&parent)
                .is_some_and(|set| set.contains(name))
    }

    fn claim(&mut self, parent: Option<i64>, name: &str) {
        self.claimed
            .entry(parent)
            .or_default()
            .insert(name.to_string());
    }

    /// 占用时按 `名字 (2)` / `名字 (3)` 往后找空位
    fn free_request_name(&mut self, parent: Option<i64>, base: &str) -> String {
        let mut candidate = base.to_string();
        let mut index = 2;
        while self.taken(parent, &candidate) {
            candidate = format!("{base} ({index})");
            index += 1;
        }
        self.claim(parent, &candidate);
        candidate
    }

    /// 定名字、算差异，产出这个接口要落到哪儿。
    fn plan_request(
        &mut self,
        draft: &RequestDraft,
        parent: Option<i64>,
        warnings: &mut Vec<ImportWarning>,
        counts: &mut ImportCounts,
    ) -> PlannedNode {
        let mut draft = draft.clone();
        draft.name = fit_name(&draft.name, &self.labels.untitled_request, warnings);

        // 名字定下来之后才知道该和谁比。借用只活在这一行，
        // 后面要改名，所以先把它摘成一个「内容是否一致」的值。
        let probe = self
            .existing
            .request(parent, &draft.name)
            .map(|found| (found.id, content_matches(found, &draft)));

        let action = match probe {
            None => {
                draft.name = self.free_request_name(parent, &draft.name);
                counts.created += 1;
                Action::Create
            }
            // 文件里两条同名项撞到同一行：第二条只能新建，
            // 否则它们互相覆盖，导入完库里还是只有一条
            Some((id, _)) if self.used_requests.contains(&id) => {
                draft.name = self.free_request_name(parent, &draft.name);
                counts.created += 1;
                Action::Create
            }
            Some((_, _)) if self.conflict == Conflict::Duplicate => {
                let original = draft.name.clone();
                draft.name = self.free_request_name(parent, &original);
                warnings.push(ImportWarning::new(
                    WarningKind::RenamedDuplicate,
                    Some(original),
                    format!(
                        "name already used in this folder; imported as `{}`",
                        draft.name
                    ),
                ));
                counts.created += 1;
                Action::Create
            }
            Some((id, true)) => {
                self.used_requests.insert(id);
                self.claim(parent, &draft.name);
                counts.skipped += 1;
                Action::Skip
            }
            Some((id, false)) => {
                self.used_requests.insert(id);
                self.claim(parent, &draft.name);
                counts.updated += 1;
                Action::Existing(id)
            }
        };

        PlannedNode::Request {
            draft: Box::new(draft),
            action,
        }
    }

    fn plan_folder(
        &mut self,
        node: &FolderNode,
        parent: Option<i64>,
        warnings: &mut Vec<ImportWarning>,
        counts: &mut ImportCounts,
    ) -> PlannedFolder {
        let name = fit_name(&node.name, &self.labels.untitled_folder, warnings);

        // 分类永远「同名即复用」：不然往里导三次就得到三层同名嵌套，
        // 侧栏会变成一串看不出差别的壳。副本模式复制的是接口，不是壳。
        let found = self.existing.folder(parent, &name);
        // `slot` = 这个分类的落脚点，子节点靠它去库里查同名
        let (action, slot, auth_to_write) = match found {
            Some(existing) => {
                self.used_folders.insert(existing.id);
                // 只有导入带了授权、且与库里不同才回写 —— 免得白改一次 updated_at
                let write = match &node.auth {
                    Some(incoming) if incoming != &existing.auth => Some(incoming.clone()),
                    _ => None,
                };
                (Action::Existing(existing.id), existing.id, write)
            }
            None => (Action::Create, self.new_slot(), node.auth.clone()),
        };
        let child_slot = Some(slot);

        let mut children = Vec::new();
        for child in &node.folders {
            children.push(PlannedNode::Folder(
                self.plan_folder(child, child_slot, warnings, counts),
            ));
        }
        for draft in &node.requests {
            children.push(self.plan_request(draft, child_slot, warnings, counts));
        }

        PlannedFolder {
            name,
            auth_to_write,
            action,
            children,
        }
    }
}

fn build_plan(
    conn: &Connection,
    source: &ImportSource,
    conflict: Conflict,
    include_environments: bool,
) -> AppResult<Plan> {
    if let Some(folder_id) = source.target_folder_id {
        ensure_usable_folder(conn, folder_id, source.workspace_id, "targetFolderId")?;
    }

    let input = match &source.file_name {
        Some(name) => ImportInput::text(&source.text).with_file_name(name),
        None => ImportInput::text(&source.text),
    };
    let (format, bundle) = importer::parse(&input)?;

    let existing = Existing::load(conn, source.workspace_id)?;
    let mut plan = Plan {
        format,
        workspace_id: source.workspace_id,
        target_folder_id: source.target_folder_id,
        nodes: Vec::new(),
        environments: Vec::new(),
        warnings: bundle.warnings.clone(),
        counts: ImportCounts::default(),
    };
    let mut ctx = Ctx {
        existing: &existing,
        conflict,
        labels: &source.labels,
        used_folders: HashSet::new(),
        used_requests: HashSet::new(),
        claimed: HashMap::new(),
        next_slot: -1,
    };

    let target = source.target_folder_id;

    // 分类排在前面，预览树读起来和侧栏一致
    for folder in &bundle.folders {
        let planned = ctx.plan_folder(folder, target, &mut plan.warnings, &mut plan.counts);
        plan.nodes.push(PlannedNode::Folder(planned));
    }
    for draft in &bundle.requests {
        let planned = ctx.plan_request(draft, target, &mut plan.warnings, &mut plan.counts);
        plan.nodes.push(planned);
    }

    if conflict == Conflict::Overwrite {
        collect_removals(&ctx, target, &mut plan.nodes, &mut plan.counts);
    }

    // 勾了才规划环境：预览也要如实反映勾选状态，
    // 否则会出现「预览说会建环境、点下去没建」
    if include_environments {
        let mut warnings = Vec::new();
        for env in &bundle.environments {
            if env.vars.is_empty() {
                continue;
            }
            let name = fit_name(&env.name, &source.labels.untitled_collection, &mut warnings);
            plan.environments.push(PlannedEnv {
                merged: existing.env_names.contains(&name),
                name,
                vars: env.vars.clone(),
            });
        }
        plan.warnings.extend(warnings);
    }

    Ok(plan)
}

/// 覆盖模式：目标范围内、文件里没提到的现存内容全都进回收站。
///
/// 走一遍**目标位置的层级**而不是把所有现存条目简单减一遍：
/// 被复用的分类要钻进去继续看（它里面的旧接口同样得清掉），
/// 没被提到的分类则整棵子树一次性处理。
fn collect_removals(
    ctx: &Ctx<'_>,
    parent: Option<i64>,
    out: &mut Vec<PlannedNode>,
    counts: &mut ImportCounts,
) {
    if let Some(folders) = ctx.existing.folders_in.get(&parent) {
        for folder in folders {
            if ctx.used_folders.contains(&folder.id) {
                // 复用的分类：它自己留着，里面的东西照样要过筛
                collect_removals(ctx, Some(folder.id), out, counts);
                continue;
            }
            out.push(PlannedNode::Folder(trash_folder(ctx, folder, counts)));
        }
    }

    if let Some(requests) = ctx.existing.requests_in.get(&parent) {
        for request in requests {
            if ctx.used_requests.contains(&request.id) {
                continue;
            }
            counts.removed += 1;
            out.push(PlannedNode::Request {
                draft: Box::new(removal_draft(request)),
                action: Action::Remove(request.id),
            });
        }
    }
}

/// 整棵子树进回收站。子节点只为了在预览里看得到 ——
/// 落库时一个 `soft_delete_tree` 就带走了，不会再逐个走一遍。
fn trash_folder(ctx: &Ctx<'_>, folder: &Folder, counts: &mut ImportCounts) -> PlannedFolder {
    let mut children = Vec::new();
    if let Some(sub) = ctx.existing.folders_in.get(&Some(folder.id)) {
        for child in sub {
            children.push(PlannedNode::Folder(trash_folder(ctx, child, counts)));
        }
    }
    if let Some(requests) = ctx.existing.requests_in.get(&Some(folder.id)) {
        for request in requests {
            counts.removed += 1;
            children.push(PlannedNode::Request {
                draft: Box::new(removal_draft(request)),
                action: Action::Remove(request.id),
            });
        }
    }

    PlannedFolder {
        name: folder.name.clone(),
        auth_to_write: None,
        action: Action::Remove(folder.id),
        children,
    }
}

/// 要进回收站的那条只在预览里露个脸，不会被落库
fn removal_draft(existing: &ApiRequest) -> RequestDraft {
    RequestDraft {
        name: existing.name.clone(),
        method: existing.method.clone(),
        url: existing.url.clone(),
        ..Default::default()
    }
}

fn to_preview(plan: &Plan) -> ImportPreview {
    ImportPreview {
        format: plan.format,
        counts: plan.counts,
        nodes: plan.nodes.iter().map(PlannedNode::preview).collect(),
        environments: plan
            .environments
            .iter()
            .map(|env| EnvPreview {
                name: env.name.clone(),
                var_count: env.vars.len(),
                merged: env.merged,
            })
            .collect(),
        warnings: plan.warnings.clone(),
    }
}

// ============================================================
// 落库
// ============================================================

fn apply_plan(conn: &mut Connection, plan: &Plan) -> AppResult<ImportOutcome> {
    // 一个事务包住全部：导入到一半失败留下半份数据，比干脆什么都不导还难收拾
    let tx = conn.transaction()?;

    for node in &plan.nodes {
        apply_node(&tx, node, plan.target_folder_id, plan.workspace_id)?;
    }
    let environments = apply_environments(&tx, &plan.environments, plan.workspace_id)?;

    tx.commit()?;

    Ok(ImportOutcome {
        format: plan.format,
        counts: plan.counts,
        environments,
        warnings: plan.warnings.clone(),
    })
}

fn apply_node(
    conn: &Connection,
    node: &PlannedNode,
    parent_id: Option<i64>,
    workspace_id: i64,
) -> AppResult<()> {
    match node {
        PlannedNode::Folder(folder) => match folder.action {
            Action::Existing(id) => {
                if let Some(auth) = &folder.auth_to_write {
                    repo::folder::set_auth(conn, id, auth)?;
                }
                apply_children(conn, &folder.children, Some(id), workspace_id)?;
            }
            Action::Create => {
                let created = repo::folder::create(conn, workspace_id, parent_id, &folder.name)?;
                if let Some(auth) = &folder.auth_to_write {
                    repo::folder::set_auth(conn, created.id, auth)?;
                }
                apply_children(conn, &folder.children, Some(created.id), workspace_id)?;
            }
            // 一棵子树一次带走，不再往里走（子节点只是预览用的样子）
            Action::Remove(id) => {
                repo::folder::soft_delete_tree(conn, id)?;
            }
            Action::Skip => {}
        },
        PlannedNode::Request { draft, action } => {
            apply_request(conn, draft, action, parent_id, workspace_id)?;
        }
    }
    Ok(())
}

fn apply_children(
    conn: &Connection,
    children: &[PlannedNode],
    parent_id: Option<i64>,
    workspace_id: i64,
) -> AppResult<()> {
    for child in children {
        apply_node(conn, child, parent_id, workspace_id)?;
    }
    Ok(())
}

fn apply_request(
    conn: &Connection,
    draft: &RequestDraft,
    action: &Action,
    parent_id: Option<i64>,
    workspace_id: i64,
) -> AppResult<()> {
    match action {
        Action::Create => {
            // 先 create 拿到 sort_order / created_at / 默认 settings，
            // 再用 save 把草稿里的字段整体覆盖上去
            let created = repo::request::create(conn, workspace_id, parent_id, &draft.name)?;
            let mut full = created;
            apply_draft(&mut full, draft);
            repo::request::save(conn, &full)?;
        }
        Action::Existing(id) => {
            // 就地更新：id / created_at / sort_order 原样留着，
            // 历史记录与 Token 源引用都还挂在它们上面
            let mut full = repo::request::get(conn, *id)?;
            apply_draft(&mut full, draft);
            repo::request::save(conn, &full)?;
        }
        Action::Skip => {}
        Action::Remove(id) => repo::request::soft_delete(conn, *id)?,
    }
    Ok(())
}

fn apply_environments(
    conn: &Connection,
    envs: &[PlannedEnv],
    workspace_id: i64,
) -> AppResult<usize> {
    let mut written = 0;
    for planned in envs {
        let environment = match repo::env::list_by_workspace(conn, workspace_id)?
            .into_iter()
            .find(|env| env.name == planned.name)
        {
            Some(found) => found,
            None => repo::env::create(conn, workspace_id, &planned.name)?,
        };
        repo::env::merge_vars(conn, environment.id, &to_env_vars(&planned.vars))?;
        written += 1;
    }
    Ok(written)
}

fn to_env_vars(vars: &[EnvVarDraft]) -> Vec<EnvVar> {
    vars.iter()
        .filter(|var| !var.key.trim().is_empty())
        .map(|var| EnvVar {
            id: 0,
            environment_id: 0,
            key: var.key.clone(),
            value: var.value.clone(),
            is_secret: var.is_secret,
            enabled: true,
            sort_order: 0,
        })
        .collect()
}

// ============================================================
// 草稿 → 行
// ============================================================

fn apply_draft(target: &mut ApiRequest, draft: &RequestDraft) {
    target.name = draft.name.clone();
    target.method = draft.method.clone();
    target.url = draft.url.clone();
    target.params = draft.params.clone();
    target.headers = draft.headers.clone();
    target.body_type = BodyType::parse(&draft.body_type);
    target.body = draft.body.clone();
    target.auth = draft.auth.clone();
    target.settings = draft.settings.clone();
    target.pre_script = draft.pre_script.clone();
    target.test_script = draft.test_script.clone();
}

/// 库里那条和文件里这条**内容**一样吗。
///
/// 只比内容，不比 id / 时间戳 / 排序 —— 那些是库自己的事。
/// 一样的话合并模式就能原地跳过，不白写一次 updated_at。
fn content_matches(existing: &ApiRequest, draft: &RequestDraft) -> bool {
    existing.name == draft.name
        && existing.method == draft.method
        && existing.url == draft.url
        && existing.params == draft.params
        && existing.headers == draft.headers
        && existing.body_type == BodyType::parse(&draft.body_type)
        && existing.body == draft.body
        && existing.auth == draft.auth
        && existing.settings == draft.settings
        && existing.pre_script == draft.pre_script
        && existing.test_script == draft.test_script
}

/// 导入时的名字处理比手工新建宽松：一份集合里有条名字超长，
/// 不该把整份文件废掉 —— 截断 + 记一条警告，其余条目照常进来。
fn fit_name(raw: &str, fallback: &str, warnings: &mut Vec<ImportWarning>) -> String {
    let trimmed = raw.trim();
    let chosen = if trimmed.is_empty() {
        fallback.trim()
    } else {
        trimmed
    };
    if chosen.chars().count() <= MAX_NAME_LEN {
        return chosen.to_string();
    }
    let cut: String = chosen.chars().take(MAX_NAME_LEN).collect();
    warnings.push(ImportWarning::new(
        WarningKind::TruncatedName,
        Some(chosen.chars().take(32).collect()),
        format!("name is longer than {MAX_NAME_LEN} characters; truncated"),
    ));
    cut
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::storage::migrate;
    use serde_json::json;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        for (id, name) in [(1_i64, "w1"), (2_i64, "w2")] {
            conn.execute(
                "INSERT INTO workspace (id, name, created_at, updated_at)
                 VALUES (?1, ?2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                rusqlite::params![id, name],
            )
            .unwrap();
        }
        conn
    }

    fn labels() -> ImportLabels {
        ImportLabels {
            untitled_folder: "未命名分类".into(),
            untitled_request: "未命名接口".into(),
            untitled_collection: "导入的集合".into(),
        }
    }

    fn source(text: &str, target: Option<i64>) -> ImportSource {
        ImportSource {
            text: text.to_string(),
            workspace_id: 1,
            target_folder_id: target,
            file_name: None,
            labels: labels(),
        }
    }

    fn preview(conn: &Connection, src: &ImportSource, conflict: Conflict) -> ImportPreview {
        to_preview(&build_plan(conn, src, conflict, true).unwrap())
    }
    fn commit(conn: &mut Connection, src: &ImportSource, conflict: Conflict) -> ImportOutcome {
        let plan = build_plan(conn, src, conflict, true).unwrap();
        apply_plan(conn, &plan).unwrap()
    }

    /// 一份两层的集合：`用户中心` 下有 `用户列表`，根上还有 `健康检查`
    fn collection(list_url: &str) -> String {
        json!({
            "info": { "name": "内部接口", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "item": [
                { "name": "用户中心", "item": [
                    { "name": "用户列表", "request": { "method": "GET", "url": list_url } }
                ]},
                { "name": "健康检查", "request": { "method": "GET", "url": "https://api.example.com/health" } }
            ]
        })
        .to_string()
    }

    fn alive(conn: &Connection) -> Vec<ApiRequest> {
        repo::request::list_by_workspace(conn, 1).unwrap()
    }

    fn find_request(conn: &Connection, name: &str) -> ApiRequest {
        alive(conn)
            .into_iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("找不到名为 {name} 的接口"))
    }

    fn request_names(conn: &Connection) -> Vec<String> {
        let mut names: Vec<String> = alive(conn).into_iter().map(|r| r.name).collect();
        names.sort();
        names
    }

    fn preview_request_names(nodes: &[PreviewNode], out: &mut Vec<String>) {
        for node in nodes {
            // 要进回收站的不算「落库的名字」
            if node.kind == PreviewKind::Request && node.status != DiffStatus::Remove {
                out.push(node.name.clone());
            }
            preview_request_names(&node.children, out);
        }
    }

    // ---- 基本落库 ----

    #[test]
    fn a_collection_lands_with_its_folders() {
        let mut conn = db();
        let outcome = commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );

        assert_eq!(outcome.counts.created, 2);
        assert_eq!(outcome.counts.updated, 0);

        let folders = repo::folder::list_by_workspace(&conn, 1).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "用户中心");

        let list = find_request(&conn, "用户列表");
        assert_eq!(list.url, "https://x/v1/users");
        assert_eq!(list.folder_id, Some(folders[0].id), "要落在分类里面");

        assert_eq!(
            find_request(&conn, "健康检查").folder_id,
            None,
            "根级请求不该被塞进分类"
        );
    }

    #[test]
    fn a_nested_folder_is_matched_under_its_own_parent() {
        let mut conn = db();
        // 目标位置已经有一个同名的顶层分类，但文件里那个是嵌在别的分类里的
        let bogus = repo::folder::create(&conn, 1, None, "订单").unwrap();

        let text = json!({
            "info": { "name": "c", "schema": "x" },
            "item": [{ "name": "用户中心", "item": [
                { "name": "订单", "item": [
                    { "name": "订单列表", "request": { "method": "GET", "url": "https://x/orders" } }
                ]}
            ]}]
        })
        .to_string();
        commit(&mut conn, &source(&text, None), Conflict::Merge);

        let folders = repo::folder::list_by_workspace(&conn, 1).unwrap();
        assert_eq!(
            folders.len(),
            3,
            "顶级那个「订单」是原有的；「用户中心」与它里面的「订单」都要新建"
        );

        let user_center = folders.iter().find(|f| f.name == "用户中心").unwrap();
        let nested = folders
            .iter()
            .find(|f| f.name == "订单" && f.parent_id == Some(user_center.id))
            .expect("嵌套分类要挂在「用户中心」下面");
        assert_ne!(nested.id, bogus.id);
        assert_eq!(find_request(&conn, "订单列表").folder_id, Some(nested.id));
    }

    #[test]
    fn a_curl_paste_lands_in_the_target_folder() {
        let mut conn = db();
        let target = repo::folder::create(&conn, 1, None, "手工分类").unwrap();

        let text = "curl 'https://api.example.com/v1/users' -H 'X-Trace: 1'";
        let outcome = commit(&mut conn, &source(text, Some(target.id)), Conflict::Merge);

        assert_eq!(outcome.format, Format::Curl);
        assert_eq!(outcome.counts.created, 1);

        let created = alive(&conn).remove(0);
        assert_eq!(created.folder_id, Some(target.id));
        assert_eq!(created.url, "https://api.example.com/v1/users");
        assert_eq!(created.name, "users", "名字由 URL 生成");
        // curl 的默认是不跟随重定向，必须写回 false
        assert!(!created.settings.follow_redirects);
    }

    // ---- 三种冲突模式 ----

    #[test]
    fn merge_updates_the_row_in_place_and_keeps_its_id() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        let before = find_request(&conn, "用户列表");

        let outcome = commit(
            &mut conn,
            &source(&collection("https://x/v2/users"), None),
            Conflict::Merge,
        );

        assert_eq!(outcome.counts.updated, 1);
        assert_eq!(outcome.counts.created, 0);

        let after = find_request(&conn, "用户列表");
        assert_eq!(after.id, before.id, "就地更新：id 必须不变");
        assert_eq!(after.created_at, before.created_at);
        assert_eq!(after.url, "https://x/v2/users");
        assert_eq!(
            alive(&conn).iter().filter(|r| r.name == "用户列表").count(),
            1,
            "不能多出一条"
        );
    }

    #[test]
    fn merge_leaves_an_unchanged_request_alone() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        let before = find_request(&conn, "用户列表");

        let outcome = commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );

        assert_eq!(outcome.counts.skipped, 2);
        assert_eq!(outcome.counts.created, 0);
        assert_eq!(outcome.counts.updated, 0);

        assert_eq!(
            find_request(&conn, "用户列表").updated_at,
            before.updated_at,
            "一模一样就不该写库"
        );
    }

    #[test]
    fn duplicate_adds_a_copy_and_keeps_the_original() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        let before = find_request(&conn, "用户列表");

        let outcome = commit(
            &mut conn,
            &source(&collection("https://x/v2/users"), None),
            Conflict::Duplicate,
        );

        assert_eq!(outcome.counts.created, 2);
        assert_eq!(outcome.counts.updated, 0);

        let copy = find_request(&conn, "用户列表 (2)");
        assert_ne!(copy.id, before.id);
        assert_eq!(copy.url, "https://x/v2/users");

        let original = find_request(&conn, "用户列表");
        assert_eq!(original.id, before.id);
        assert_eq!(original.url, before.url, "原条目必须一根汗毛不动");
    }

    #[test]
    fn overwrite_moves_unmentioned_requests_to_the_trash() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        let doomed = find_request(&conn, "健康检查");

        // 同一份集合，只是把根上那条「健康检查」删了
        let only_list = json!({
            "info": { "name": "c", "schema": "x" },
            "item": [{ "name": "用户中心", "item": [
                { "name": "用户列表", "request": { "method": "GET", "url": "https://x/v1/users" } }
            ]}]
        })
        .to_string();
        let outcome = commit(&mut conn, &source(&only_list, None), Conflict::Overwrite);

        assert_eq!(outcome.counts.removed, 1);
        assert_eq!(outcome.counts.skipped, 1);
        assert_eq!(alive(&conn).len(), 1);

        // 软删除：回收站里找得到，还能还原
        let trashed = repo::request::list_deleted(&conn, 1).unwrap();
        assert_eq!(trashed.len(), 1);
        assert_eq!(trashed[0].id, doomed.id);
        repo::request::restore(&conn, doomed.id).unwrap();
        assert_eq!(alive(&conn).len(), 2);
    }

    #[test]
    fn overwrite_removes_a_whole_unmentioned_folder_subtree() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );

        let health_only = json!({
            "info": { "name": "c", "schema": "x" },
            "item": [{ "name": "健康检查", "request": { "method": "GET", "url": "https://api.example.com/health" } }]
        })
        .to_string();
        let outcome = commit(&mut conn, &source(&health_only, None), Conflict::Overwrite);

        assert_eq!(outcome.counts.removed, 1, "被移除的是分类里的那条");
        assert!(repo::folder::list_by_workspace(&conn, 1)
            .unwrap()
            .is_empty());
        assert_eq!(repo::folder::list_deleted(&conn, 1).unwrap().len(), 1);
    }

    #[test]
    fn overwrite_scope_stops_at_the_target_folder() {
        let mut conn = db();
        let mine = repo::folder::create(&conn, 1, None, "我的分类").unwrap();
        let theirs = repo::folder::create(&conn, 1, None, "别人的分类").unwrap();
        let keep = repo::request::create(&conn, 1, Some(theirs.id), "不该动").unwrap();

        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), Some(mine.id)),
            Conflict::Overwrite,
        );

        assert!(
            repo::request::get(&conn, keep.id).is_ok(),
            "范围外的必须原封不动"
        );
        assert_eq!(alive(&conn).len(), 3);
    }

    #[test]
    fn overwrite_leaves_another_workspace_alone() {
        let mut conn = db();
        let other = repo::folder::create(&conn, 2, None, "别的空间").unwrap();
        let theirs = repo::request::create(&conn, 2, Some(other.id), "别的接口").unwrap();

        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Overwrite,
        );

        assert!(repo::request::get(&conn, theirs.id).is_ok());
        assert_eq!(repo::request::list_by_workspace(&conn, 2).unwrap().len(), 1);
    }

    // ---- 幂等与复名 ----

    #[test]
    fn folders_are_reused_so_importing_twice_does_not_nest() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );

        let folders = repo::folder::list_by_workspace(&conn, 1).unwrap();
        assert_eq!(folders.len(), 1, "同名分类要复用，不能再建一个");
        assert_eq!(folders[0].parent_id, None);
        assert_eq!(alive(&conn).len(), 2);
    }

    #[test]
    fn duplicate_mode_also_reuses_folders() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Duplicate,
        );
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Duplicate,
        );

        assert_eq!(
            repo::folder::list_by_workspace(&conn, 1).unwrap().len(),
            1,
            "副本模式复制的是接口，不是壳"
        );
        assert_eq!(alive(&conn).len(), 4);
    }

    #[test]
    fn a_trashed_request_is_not_treated_as_existing() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        let original = find_request(&conn, "健康检查");
        repo::request::soft_delete(&conn, original.id).unwrap();

        let outcome = commit(
            &mut conn,
            &source(&collection("https://x/v2/users"), None),
            Conflict::Merge,
        );

        // 回收站里的那条压根不算「已存在」：同名的新条目要另起一行，
        // 而不是把一条已经进回收站的记录原地改写
        assert_eq!(outcome.counts.updated, 1, "用户列表改了地址");
        assert_eq!(outcome.counts.created, 1, "健康检查是全新的");

        let fresh = find_request(&conn, "健康检查");
        assert_ne!(fresh.id, original.id);
        assert_eq!(repo::request::list_deleted(&conn, 1).unwrap().len(), 1);
    }

    // ---- 预览与落库必须一致 ----

    #[test]
    fn preview_names_match_what_actually_lands() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        let before = request_names(&conn);

        let src = source(&collection("https://x/v2/users"), None);
        let shown = preview(&conn, &src, Conflict::Duplicate);
        let mut shown_names: Vec<String> = Vec::new();
        preview_request_names(&shown.nodes, &mut shown_names);
        shown_names.sort();

        let outcome = commit(&mut conn, &src, Conflict::Duplicate);

        let added: Vec<String> = request_names(&conn)
            .into_iter()
            .filter(|name| !before.contains(name))
            .collect();

        assert_eq!(shown_names, added, "预览树上的名字就是最终落库的名字");
        assert_eq!(outcome.counts.created, shown.counts.created);
    }

    #[test]
    fn preview_counts_what_would_be_removed() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );

        let health_only = json!({
            "info": { "name": "c", "schema": "x" },
            "item": [{ "name": "健康检查", "request": { "method": "GET", "url": "https://api.example.com/health" } }]
        })
        .to_string();
        let shown = preview(&conn, &source(&health_only, None), Conflict::Overwrite);

        assert_eq!(shown.counts.removed, 1, "被移除的分类里那一条");
        assert_eq!(shown.counts.skipped, 1);
        // 预览不写库
        assert_eq!(alive(&conn).len(), 2);
    }

    #[test]
    fn an_import_matching_the_file_exactly_changes_nothing() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Merge,
        );
        let outcome = commit(
            &mut conn,
            &source(&collection("https://x/v1/users"), None),
            Conflict::Overwrite,
        );

        assert_eq!(outcome.counts.created, 0);
        assert_eq!(outcome.counts.updated, 0);
        assert_eq!(outcome.counts.removed, 0);
        assert_eq!(outcome.counts.skipped, 2);
    }

    // ---- 名字 ----

    #[test]
    fn nameless_things_get_the_frontend_labels() {
        let mut conn = db();
        let text = json!({
            "info": { "name": "c", "schema": "x" },
            "item": [{ "item": [{ "request": { "method": "GET", "url": "https://x/v1/users" } }] }]
        })
        .to_string();
        commit(&mut conn, &source(&text, None), Conflict::Merge);

        let folders = repo::folder::list_by_workspace(&conn, 1).unwrap();
        assert_eq!(folders[0].name, "未命名分类", "空名要用前端送的占位文案");
    }

    #[test]
    fn a_too_long_name_is_truncated_with_a_warning() {
        let mut conn = db();
        let long = "长".repeat(MAX_NAME_LEN + 20);
        let text = json!({
            "info": { "name": "c", "schema": "x" },
            "item": [{ "name": long, "request": { "method": "GET", "url": "https://x/v1/users" } }]
        })
        .to_string();
        let outcome = commit(&mut conn, &source(&text, None), Conflict::Merge);

        assert!(outcome
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::TruncatedName));
        assert_eq!(alive(&conn)[0].name.chars().count(), MAX_NAME_LEN);
    }

    #[test]
    fn duplicate_names_inside_one_file_keep_both() {
        let mut conn = db();
        let text = json!({
            "info": { "name": "c", "schema": "x" },
            "item": [
                { "name": "列表", "request": { "method": "GET", "url": "https://x/a" } },
                { "name": "列表", "request": { "method": "GET", "url": "https://x/b" } }
            ]
        })
        .to_string();
        let outcome = commit(&mut conn, &source(&text, None), Conflict::Merge);

        assert_eq!(outcome.counts.created, 2);
        assert_eq!(request_names(&conn), vec!["列表", "列表 (2)"]);
    }

    // ---- 环境 ----

    fn with_variables(name: &str, vars: serde_json::Value) -> String {
        json!({
            "info": { "name": name, "schema": "x" },
            "variable": vars,
            "item": []
        })
        .to_string()
    }

    #[test]
    fn collection_variables_become_an_environment() {
        let mut conn = db();
        let text = with_variables(
            "内部接口",
            json!([{ "key": "baseUrl", "value": "https://x" }]),
        );
        let outcome = commit(&mut conn, &source(&text, None), Conflict::Merge);

        assert_eq!(outcome.environments, 1);
        let envs = repo::env::list_by_workspace(&conn, 1).unwrap();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "内部接口");
        let vars = repo::env::list_vars(&conn, envs[0].id).unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "baseUrl");
        assert_eq!(vars[0].value, "https://x");
    }

    #[test]
    fn a_second_import_merges_into_the_same_environment() {
        let mut conn = db();
        commit(
            &mut conn,
            &source(
                &with_variables(
                    "内部接口",
                    json!([{ "key": "baseUrl", "value": "https://x" }]),
                ),
                None,
            ),
            Conflict::Merge,
        );

        // 用户自己在变量表里加了一条
        let env = repo::env::list_by_workspace(&conn, 1).unwrap().remove(0);
        let mut manual = repo::env::list_vars(&conn, env.id).unwrap();
        manual.push(EnvVar {
            id: 0,
            environment_id: env.id,
            key: "手工加的".into(),
            value: "1".into(),
            is_secret: false,
            enabled: true,
            sort_order: 0,
        });
        repo::env::replace_vars(&mut conn, env.id, &manual).unwrap();

        let second = with_variables(
            "内部接口",
            json!([
                { "key": "baseUrl", "value": "https://y" },
                { "key": "token", "value": "t" }
            ]),
        );
        let outcome = commit(&mut conn, &source(&second, None), Conflict::Merge);

        assert_eq!(outcome.environments, 1);
        assert_eq!(
            repo::env::list_by_workspace(&conn, 1).unwrap().len(),
            1,
            "同名环境要并进去，不能多一个"
        );
        let vars = repo::env::list_vars(&conn, env.id).unwrap();
        let keys: Vec<&str> = vars.iter().map(|v| v.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["baseUrl", "手工加的", "token"],
            "同名的改值，新的追加"
        );
        assert_eq!(vars[0].value, "https://y");
    }

    #[test]
    fn environments_are_optional() {
        let mut conn = db();
        let text = with_variables(
            "内部接口",
            json!([{ "key": "baseUrl", "value": "https://x" }]),
        );
        let src = source(&text, None);

        let shown = preview(&conn, &src, Conflict::Merge);
        assert_eq!(shown.environments.len(), 1, "预览要照实列出有哪些环境");

        let plan = build_plan(&conn, &src, Conflict::Merge, false).unwrap();
        let outcome = apply_plan(&mut conn, &plan).unwrap();

        assert_eq!(outcome.environments, 0);
        assert!(repo::env::list_by_workspace(&conn, 1).unwrap().is_empty());
    }

    // ---- 坏输入 ----

    #[test]
    fn a_broken_target_folder_is_refused_before_anything_is_written() {
        let conn = db();
        let trashed = repo::folder::create(&conn, 1, None, "已删").unwrap();
        repo::folder::soft_delete_tree(&conn, trashed.id).unwrap();
        let foreign = repo::folder::create(&conn, 2, None, "别的空间").unwrap();

        for bad in [trashed.id, foreign.id, 9999] {
            let err = build_plan(
                &conn,
                &source(&collection("https://x/v1/users"), Some(bad)),
                Conflict::Merge,
                true,
            )
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    AppError::InvalidInput { .. } | AppError::NotFound { .. }
                ),
                "落点 #{bad} 不该被接受"
            );
        }
        assert!(alive(&conn).is_empty());
    }

    #[test]
    fn unreadable_input_surfaces_the_detect_error() {
        let conn = db();
        let err = build_plan(
            &conn,
            &source("just some words", None),
            Conflict::Merge,
            true,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::error::codes::IMPORT_DETECT);
    }
}
