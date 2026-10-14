//! 一次性数据播种：给已有请求铺上各自的 Token 源。
//!
//! 只由 [`crate::storage::migrate::run`] 在 008 首次应用时调用**一次** ——
//! 它不是常驻行为，所以「用户手工删掉某条 Token 源」不会被下次启动重新长回来。
//!
//! ## 这是一次性迁移，不是常驻功能
//!
//! **播种本身不做任何域名判断**，而是由调用方（`migrate.rs`）传入允许播种的
//! host 白名单。为空 = 一条都不铺。
//!
//! 为什么要做成参数而不是内置常量：本仓库最初是从一份**特定的业务库**上演进
//! 出来的，008 那次迁移要照顾的正是那份库里的既有请求。把它固化成模块级常量
//! 会让「换个人装上就跑」这件事里带上一份不属于他的域名清单 —— 既无意义，
//! 也是不该出现在通用代码里的东西。**白名单从哪来由部署方决定，模块只管执行。**
//!
//! ## 为什么这件事不写在 008 的 SQL 里
//!
//! 判据要读**取 token 请求的 scope**，而 scope 藏在请求体里、有三种存法
//! （`form_fields` / `body_rows` / `body` 原文，取决于请求体类型与写入年代）。
//! 还要从 URL 里切 host 与服务名、把 scope 按空白切词、做 percent-decode。
//! SQLite 没有 `split`，这些用 SQL 表达出来没人敢改；换成一段 Rust 纯函数，
//! 它就能被单测逐条钉住。
//!
//! ## 播种规则（与域名无关的那部分）
//!
//! 1. **每条请求各自一份源**，归属 `request_id`；源之间不共享 token 与倒计时。
//! 2. **同 host 配对**：业务接口与取 token 请求必须在**同一个 host** 上，
//!    且该 host 在传入的白名单里。
//!
//!    ⚠️ 判据是 **host 全等**，不是「URL 里包含 `api.xx`」这种子串判断：
//!    `api.example.com` 与 `api.example.cn` 都含 `api`，子串判断会把它们
//!    一起吞进来。
//!
//!    （历史：这一条原先只写「host 相同才配对」，还断言若干域名
//!    「没有同 host 的取 token 请求」—— 那个前提是**假的**，于是真库上
//!    多铺了一批。009 的注释里记了这件事。）
//! 3. **scope 覆盖才用**：接口 URL 的**第二个路径段**是它要的服务名
//!    （`/PSA/MasterData2/v1/…` → `MasterData2`），取 token 请求的 `scope`
//!    里列出它才拿得到可用 token。
//! 4. **环境默认候选**：同一域名组里，名字含「测试」或「生产」的那几条里
//!    **scope 词数最少**的（词少 = 权限最小且够用）。
//! 5. **默认覆盖就用默认**；默认不覆盖时退到覆盖集里 scope 词数最少的；
//!    谁都不覆盖时仍用默认。

use rusqlite::Connection;

use crate::error::AppResult;
use crate::storage::now_iso;

/// host 是否在允许播种的白名单里。
///
/// 判据必须是 **host 全等**：`api.example.com` 与 `apidemo.example.com`
/// 都含 `api`，用「URL 包含 `api.xx`」这种子串判断会把它们一起吞进来。
///
/// 白名单为空时**全部拒绝** —— 这是默认行为，也是新装用户该有的行为。
fn is_rule_host(host: &str, rule_hosts: &[String]) -> bool {
    rule_hosts.iter().any(|h| h == host)
}

/// 一个候选（「取 token 请求」）的解析结果。
struct Candidate {
    id: i64,
    /// 名字里含「测试」或「生产」—— 组内默认候选的标记（见模块注释第 4 条）
    named: bool,
    /// scope 词，已小写
    scope: Vec<String>,
}

/// 一条待铺源的业务请求。
struct Target {
    id: i64,
    workspace_id: i64,
    name: String,
    url: String,
}

/// 给所有已有请求铺 Token 源，返回新建的行数。
///
/// `rule_hosts` 是允许播种的 host 白名单，**为空则一条都不铺**。
/// 这是一次性迁移用的函数，不是常驻特性。
///
/// 幂等：靠 `token_source.request_id` 上的唯一索引 + `INSERT OR IGNORE`，
/// 重复执行不会产生第二行，也不会覆盖用户改过的配置。
pub fn request_token_sources(conn: &Connection, rule_hosts: &[String]) -> AppResult<usize> {
    // 白名单为空是最常见的情况（新装用户），早退省掉整趟查询。
    if rule_hosts.is_empty() {
        return Ok(0);
    }

    let mut stmt = conn.prepare(
        "SELECT id, workspace_id, name, url, body, body_rows, form_fields
           FROM request
          WHERE deleted_at IS NULL
          ORDER BY id",
    )?;

    let rows: Vec<(i64, i64, String, String, String, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let mut candidates: Vec<(i64, String, Candidate)> = Vec::new(); // (workspace, host, cand)
    let mut targets: Vec<(String, Target)> = Vec::new(); // (host, target)

    for (id, workspace_id, name, url, body, body_rows, form_fields) in rows {
        let Some(host) = host_of(&url) else { continue };
        // 闸门放在**取候选之前**：不在白名单上的请求既不当候选、也不当待铺源。
        if !is_rule_host(&host, rule_hosts) {
            continue;
        }
        let is_fetch = url.to_ascii_lowercase().contains("token");

        if is_fetch {
            candidates.push((
                workspace_id,
                host,
                Candidate {
                    id,
                    // 名字里带环境词 = 用户在名字上认过的「这个环境就拿它」
                    named: name.contains("测试") || name.contains("生产"),
                    scope: scope_words(&body, &body_rows, &form_fields),
                },
            ));
        } else {
            targets.push((
                host,
                Target {
                    id,
                    workspace_id,
                    name,
                    url,
                },
            ));
        }
    }

    let now = now_iso();
    let mut insert = conn.prepare(
        "INSERT OR IGNORE INTO token_source
           (workspace_id, request_id, name, fetch_request_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
    )?;

    let mut created = 0usize;
    for (host, target) in &targets {
        let group: Vec<&Candidate> = candidates
            .iter()
            .filter(|(ws, h, _)| *ws == target.workspace_id && h == host)
            .map(|(_, _, c)| c)
            .collect();
        if group.is_empty() {
            continue;
        }

        let Some(fetch) = pick(&group, service_of(&target.url).as_deref()) else {
            continue;
        };

        created += insert.execute(rusqlite::params![
            target.workspace_id,
            target.id,
            target.name,
            fetch,
            now,
        ])?;
    }

    Ok(created)
}

/// 009 的数据变动：把 008 铺到「白名单之外」的源收回来。
///
/// 008 的判据当时只写了「业务 host == 取 token host」，没把域名白名单摆进去，
/// 于是那些同 host 但不在范围内的请求也铺上了。真库实测多出一批，
/// 而 008 自己的文件头写的末态数比它少一截。
///
/// 为什么是「删掉多铺的」而不是「清空重铺」：重铺会连 `token_cache` 一起清掉，
/// 而缓存行的 `token_source_id` 在新表里指向的是**另一个接口的新源**
/// （旧 id 被新行顶掉）—— 不清就是张冠李戴，清就是白丢一批还能用的 token。
/// 收窄删除范围两个问题都没有。
///
/// 判据复用 [`is_rule_host`]，与播种**同一处定义**：两处各写一遍的话，
/// 下次加域名必然只改一处，另一处会静默地继续按旧范围工作。
///
/// ⚠️ `rule_hosts` 为空时**不动任何行**（早退）。这与播种侧对称：
/// 一个没配白名单的部署方，「收回来」这个动作没有依据 ——
/// 它手上根本没有一份「该留什么」的清单。
pub fn drop_sources_outside_rule_hosts(
    conn: &Connection,
    rule_hosts: &[String],
) -> AppResult<usize> {
    if rule_hosts.is_empty() {
        return Ok(0);
    }

    let mut stmt = conn.prepare(
        "SELECT ts.id, r.url\n           FROM token_source ts\n           JOIN request r ON r.id = ts.request_id",
    )?;
    let existing: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let mut removed = 0usize;
    for (id, url) in existing {
        // 判据与播种同一句话：只有白名单上的接口才允许有源。
        // 解析不出 host 的（例如空 URL）当然也不算 —— 播种那边同样不会给它建。
        let allowed = matches!(host_of(&url), Some(h) if is_rule_host(&h, rule_hosts));
        if !allowed {
            removed += conn.execute("DELETE FROM token_source WHERE id = ?1", [id])?;
        }
    }
    Ok(removed)
}

/// 组内选一条来取 token。
///
/// 排序键是「scope 词数 → id」：词数少说明这条是**干净的环境级凭据**，
/// 词数多说明它还额外带了某个服务的 scope（`#205` 那种）。
/// 用 `min_by_key` 而不是 `min_by` + 比较闭包，是因为 `group` 是 `&[&Candidate]`，
/// `min_by` 的闭包要接 `&&&Candidate` —— 绕这个签名只会把代码写成噪音。
fn pick(group: &[&Candidate], service: Option<&str>) -> Option<i64> {
    fn rank(c: &Candidate) -> (usize, i64) {
        (c.scope.len(), c.id)
    }

    // ④ 环境默认候选：名字被声明过的那几条里 scope 最少的；一条都没有则退化为全组最少
    let default = group
        .iter()
        .filter(|c| c.named)
        .min_by_key(|c| rank(c))
        .or_else(|| group.iter().min_by_key(|c| rank(c)))
        .copied()?;

    let Some(service) = service else {
        return Some(default.id);
    };

    let covers = |c: &Candidate| c.scope.iter().any(|w| w == service);

    // ⑤ 默认覆盖就用默认
    if covers(default) {
        return Some(default.id);
    }
    // 默认不覆盖 → 覆盖集里 scope 词数最少的
    if let Some(best) = group.iter().filter(|c| covers(c)).min_by_key(|c| rank(c)) {
        return Some(best.id);
    }
    // 谁都不覆盖 → 仍用默认（拿到的 token 至少是本的，缺 scope 是数据问题，
    // 不该由播种静默改成"另一个环境/另一套凭据"）
    Some(default.id)
}

/// `scheme://host[:port]/…` → `host`（小写，无端口）。
///
/// 容忍没有 scheme 的 URL（有人会从文档里直接复制 `example.com/api` 这种裸写法）。
fn host_of(url: &str) -> Option<String> {
    // `split_once` 而不是 `split(..).next_back()`：`&str` 当分隔符时
    // `Split` **不是** `DoubleEndedIterator`，`next_back` 编译不过。
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.split(':').next().unwrap_or(authority);
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// URL 的**第二个路径段**（第一段是产品域）：`/PSA/MasterData2/v1/…` → `masterdata2`。
///
/// 第二段是服务名，也正是 scope 里列的那些词。取不到就不做 scope 判断。
fn service_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let path = rest.split_once('/')?.1;

    let mut segments = path.split('/').filter(|s| !s.is_empty());
    segments.next()?; // 跳过产品域（第一段路径）
    segments.next().map(|s| s.to_ascii_lowercase())
}

/// 从请求体里抽 `scope` 并切成词（小写）。
///
/// 三种存法都要试，因为同一张表里压着三代写法：
/// * `form_fields` —— 005 之后的 `multipart/form-data` 字段行
/// * `body_rows` —— `application/x-www-form-urlencoded` 的行
/// * `body` 原文 —— 老请求只有这一列（`#170` 的 `form` 体就在这儿，多行 `k=v`）
///
/// 顺序按"结构化 → 原文"，取第一个非空结果。scope 的值里空格可能是
/// `%20`（urlencoded）或字面空格（form），两种都归一。
fn scope_words(body: &str, body_rows: &str, form_fields: &str) -> Vec<String> {
    let raw = key_value(form_fields, "scope")
        .or_else(|| key_value(body_rows, "scope"))
        .or_else(|| raw_scope(body));

    let Some(raw) = raw else { return Vec::new() };
    percent_decode(&raw)
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// 从 `[{"key":…,"value":…}]` 里取某个键的值。
fn key_value(json: &str, key: &str) -> Option<String> {
    let items: serde_json::Value = serde_json::from_str(json).ok()?;
    items.as_array()?.iter().find_map(|item| {
        let k = item.get("key")?.as_str()?;
        if k.eq_ignore_ascii_case(key) {
            Some(item.get("value")?.as_str()?.to_string())
        } else {
            None
        }
    })
}

/// 从 `k=v&k=v` 或按行分隔的 `k=v` 原文里取某个键的值。
fn raw_scope(body: &str) -> Option<String> {
    body.split(['&', '\n'])
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("scope"))
        .map(|(_, v)| v.trim().to_string())
}

/// `%XX` 与 `+` 的解码。不引第三方 crate —— 这里只需要这两个规则。
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(hi * 16 + lo);
                        i += 3;
                    }
                    // 不是合法转义就原样留下：宁可让这个 scope 词匹配不上，
                    // 也不要吃掉字符把它变成另一个词
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
