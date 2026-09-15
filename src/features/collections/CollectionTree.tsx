import {
  ChevronRight,
  Copy,
  CornerDownRight,
  Folder as FolderIcon,
  FolderOpen,
  FolderPlus,
  KeyRound,
  Layers,
  Pencil,
  Plus,
  SearchX,
  Trash,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { validateName } from "@/lib/name";
import type { ApiRequest, Folder } from "@/lib/types";
import { ContextMenu, type MenuItem } from "@/features/ui/ContextMenu";
import { ConfirmDialog, PromptDialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { MoveDialog } from "@/features/collections/MoveDialog";
import { useRequestStore } from "@/store/request";
import { useUiStore } from "@/store/ui";
import {
  childFolders,
  folderSubtree,
  requestsInFolder,
  useWorkspaceStore,
} from "@/store/workspace";

// ---------------------------------------------------------------
// 常量与类型
// ---------------------------------------------------------------

/** 缩进基准（px）。深度乘它 —— 比写 6 层 `pl-*` 类干净 */
const INDENT = 11;

/** 自定义 MIME。用私有类型名，避免被别的拖放源（文件、文本）误接 */
const DRAG_MIME = "application/x-reqly-node";

interface NodeRef {
  kind: "folder" | "request";
  id: number;
}

/**
 * 正在进行的拖拽。
 *
 * `forbidden` 在 **dragstart 那一刻**就算好并冻结全程 ——
 * 「不能移到自己的子孙下」这条规则必须在这时判完。等 dragover
 * 再逐帧算，不仅白算，还会让非法目标在拖拽途中一直显示投放高亮，
 * 最后关头才被拒（用户会以为工具坏了）。
 */
interface DragState extends NodeRef {
  forbidden: ReadonlySet<number>;
}

/**
 * 投放目标。
 *
 * 三种取值对应三种落点：某个分类、根级（未分类），
 * 以及 `null`（没有目标）。
 *
 * 注意「拖到**请求行**上」也算投给该请求所在的分类 —— 否则用户必须
 * 精确命中分类那一行才能放进去，一个分类里装了 20 个接口时极难操作。
 */
type DropTarget = number | "root";

type MenuTarget =
  | { kind: "folder"; folder: Folder }
  | { kind: "request"; request: ApiRequest }
  | { kind: "blank" };

/**
 * 集合树。
 *
 * 三个实现选择：
 * 1. **先拍平成行再渲染**（而不是递归组件）。递归渲染在折叠/筛选时会不断
 *    重建整棵 React 子树；拍平之后平铺 map，每一行的 key 稳定，
 *    折叠一个文件夹只影响它后面那段。行数上千时差别很明显。
 * 2. **折叠状态存 id 集合**（而不是节点上的 boolean）。后端返回的 Folder
 *    没有 UI 字段，且 `refresh()` 之后是新对象 —— 状态挂在 id 上才不会被刷新冲掉。
 * 3. **就地重命名**（而不是弹对话框）。改名字是最频繁的整理动作，
 *    弹窗会打断视线；行内编辑让「改完看一眼旁边」不需要额外动作。
 *    副作用是必须自己处理 Enter / Esc / 失焦三种收尾 —— 见 `RenameField`。
 */
export function CollectionTree() {
  const { t } = useI18n();
  const folders = useWorkspaceStore((s) => s.folders);
  const requests = useWorkspaceStore((s) => s.requests);
  const activeRequestId = useUiStore((s) => s.activeRequestId);
  const filter = useUiStore((s) => s.filter);

  const [collapsed, setCollapsed] = useState<ReadonlySet<number>>(() => new Set<number>());
  const [menu, setMenu] = useState<{ x: number; y: number; target: MenuTarget } | null>(null);
  const [renaming, setRenaming] = useState<NodeRef | null>(null);
  const [moving, setMoving] = useState<{ node: NodeRef; name: string } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<
    { node: NodeRef; name: string; folders: number; requests: number } | null
  >(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [dropTarget, setDropTarget] = useState<DropTarget | null>(null);

  const toggle = useCallback((id: number) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const rows = useMemo(
    () => flatten(folders, requests, collapsed, filter),
    [folders, requests, collapsed, filter],
  );

  /** 删除确认要显示影响范围；本地平铺数据就是全量存活集合，能算准 */
  const askDelete = useCallback(
    (node: NodeRef, name: string) => {
      const subtree = node.kind === "folder" ? folderSubtree(folders, node.id) : null;
      setConfirmDelete({
        node,
        name,
        folders: subtree ? subtree.size - 1 : 0, // 减去自己
        requests: subtree
          ? requests.filter((r) => r.folderId !== null && subtree.has(r.folderId)).length
          : 0,
      });
    },
    [folders, requests],
  );

  // ---------------------------------------------------------------
  // 拖拽
  // ---------------------------------------------------------------

  const beginDrag = useCallback(
    (event: React.DragEvent, node: NodeRef) => {
      const forbidden =
        node.kind === "folder" ? folderSubtree(folders, node.id) : new Set<number>();
      setDrag({ ...node, forbidden });
      // 某些浏览器不调 setData 就不会真正启动拖拽
      event.dataTransfer.setData(DRAG_MIME, JSON.stringify(node));
      event.dataTransfer.setData("text/plain", String(node.id));
      event.dataTransfer.effectAllowed = "move";
    },
    [folders],
  );

  const endDrag = useCallback(() => {
    setDrag(null);
    setDropTarget(null);
  }, []);

  const isAcceptable = useCallback(
    (target: DropTarget): boolean => {
      if (!drag) return false;
      if (target === "root") return true;
      // 目标落在被拖节点的子树里 → 会形成环
      if (drag.forbidden.has(target)) return false;
      return true; // 拖回原地是空操作，无害，不拦
    },
    [drag],
  );

  /**
   * 只有**合法目标**才 `preventDefault()`。
   *
   * 不 preventDefault 时浏览器会自己显示「禁止投放」，我们的高亮也不会出现 ——
   * 「非法目标不显示投放区」这条要求靠这个实现，不需要额外画红框。
   */
  const dragOver = useCallback(
    (event: React.DragEvent, target: DropTarget) => {
      if (!isAcceptable(target)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setDropTarget(target);
    },
    [isAcceptable],
  );

  const drop = useCallback(
    (event: React.DragEvent, target: DropTarget) => {
      if (!isAcceptable(target)) return;
      event.preventDefault();
      const node = drag;
      setDrag(null);
      setDropTarget(null);
      if (!node) return;

      const folderId = target === "root" ? null : target;
      const store = useWorkspaceStore.getState();
      if (node.kind === "folder") void store.moveFolder(node.id, folderId);
      else void store.moveRequest(node.id, folderId);
    },
    [drag, isAcceptable],
  );

  const clearDrop = useCallback((target: DropTarget) => {
    setDropTarget((prev) => (prev === target ? null : prev));
  }, []);

  // ---------------------------------------------------------------
  // 菜单
  // ---------------------------------------------------------------

  const openMenu = useCallback(
    (event: React.MouseEvent | React.KeyboardEvent, target: MenuTarget) => {
      event.preventDefault();
      event.stopPropagation();

      // 鼠标事件有视口坐标；键盘事件（Shift+F10 / 菜单键）没有，
      // 退回用被操作那一行的位置顶替，效果是「菜单出现在这一行下方」
      if ("clientX" in event) {
        setMenu({ x: event.clientX, y: event.clientY, target });
      } else {
        const rect = (event.target as HTMLElement).getBoundingClientRect();
        setMenu({ x: rect.left + 24, y: rect.bottom, target });
      }
    },
    [],
  );

  const menuItems = useMemo((): MenuItem[] => {
    if (!menu) return [];
    const { target } = menu;

    if (target.kind === "blank") {
      return [
        {
          key: "new-request",
          label: t("collections.newRequest"),
          icon: Plus,
          onSelect: () => useUiStore.getState().openCreateDialog("request", null),
        },
        {
          key: "new-folder",
          label: t("sidebar.newFolder"),
          icon: FolderPlus,
          onSelect: () => useUiStore.getState().openCreateDialog("folder", null),
        },
      ];
    }

    if (target.kind === "folder") {
      const folder = target.folder;
      const store = useWorkspaceStore.getState();
      return [
        {
          key: "new-request",
          label: t("collections.newRequestIn"),
          icon: Plus,
          onSelect: () => useUiStore.getState().openCreateDialog("request", folder.id),
        },
        {
          key: "new-folder",
          label: t("collections.newSubfolder"),
          icon: FolderPlus,
          onSelect: () => useUiStore.getState().openCreateDialog("folder", folder.id),
        },
        {
          key: "token-source",
          label: folder.isTokenSource
            ? t("collections.unmarkTokenSource")
            : t("collections.markTokenSource"),
          icon: KeyRound,
          separatorBefore: true,
          onSelect: () => void store.setFolderTokenSource(folder.id, !folder.isTokenSource),
        },
        {
          key: "rename",
          label: t("common.rename"),
          icon: Pencil,
          separatorBefore: true,
          onSelect: () => setRenaming({ kind: "folder", id: folder.id }),
        },
        {
          key: "move",
          label: t("collections.moveTo"),
          icon: CornerDownRight,
          onSelect: () => setMoving({ node: { kind: "folder", id: folder.id }, name: folder.name }),
        },
        {
          key: "delete",
          label: t("common.delete"),
          icon: Trash,
          danger: true,
          separatorBefore: true,
          onSelect: () => askDelete({ kind: "folder", id: folder.id }, folder.name),
        },
      ];
    }

    const request = target.request;
    return [
      {
        key: "rename",
        label: t("common.rename"),
        icon: Pencil,
        onSelect: () => setRenaming({ kind: "request", id: request.id }),
      },
      {
        key: "duplicate",
        label: t("collections.duplicate"),
        icon: Copy,
        onSelect: () => {
          const name = t("collections.duplicateSuffix", { name: request.name });
          void useWorkspaceStore
            .getState()
            .duplicateRequest(request.id, name)
            // 复制完直接选中副本 —— 「刚复制的那份」才是用户接下来要改的东西
            .then((copy) => openInEditor(copy));
        },
      },
      {
        key: "move",
        label: t("collections.moveTo"),
        icon: CornerDownRight,
        onSelect: () =>
          setMoving({ node: { kind: "request", id: request.id }, name: request.name }),
      },
      {
        key: "delete",
        label: t("common.delete"),
        icon: Trash,
        danger: true,
        separatorBefore: true,
        onSelect: () => askDelete({ kind: "request", id: request.id }, request.name),
      },
    ];
  }, [menu, t, askDelete]);

  // ---------------------------------------------------------------
  // 渲染
  // ---------------------------------------------------------------

  return (
    <>
      <ul
        role="tree"
        className="flex min-h-full flex-col gap-px"
        // 空白处右键 → 在根级新建。行上的右键会 stopPropagation，不会走到这里
        onContextMenu={(e) => openMenu(e, { kind: "blank" })}
      >
        {/* 有请求、但一个分类都没有时，下面那堆请求全是平铺的 ——
            而且「未归档」分组标题按设计不会出现（见 flatten），
            用户看到的就是一坨没有归属的条目，也永远配不上 Token。
            这里补一条引导，说明为什么该建分类、并给个直达入口。 */}
        {folders.length === 0 && requests.length > 0 && filter.trim() === "" ? (
          <li className="mb-1 rounded-md border border-dashed border-line bg-canvas-subtle/60 px-3 py-2.5">
            <p className="text-[12px] font-medium text-ink-muted">{t("collections.noFolders")}</p>
            <p className="mt-0.5 text-[11px] leading-relaxed text-ink-faint">
              {t("collections.noFoldersHint")}
            </p>
            <button
              type="button"
              onClick={() => useUiStore.getState().openCreateDialog("folder", null)}
              className="mt-2 flex items-center gap-1 text-[11px] text-brand transition-colors duration-100 hover:underline"
            >
              <FolderPlus size={11} strokeWidth={2.2} aria-hidden />
              {t("sidebar.newFolder")}
            </button>
          </li>
        ) : null}
        {rows.length === 0 ? (
          <li>
            {/* 「库里是空的」和「筛出来是空的」要分开说 ——
                否则用户会以为自己的东西没了 */}
            {filter.trim() === "" ? (
              <EmptyState
                icon={Layers}
                title={t("common.empty")}
                description={t("sidebar.emptyCollections")}
                className="mt-8"
              />
            ) : (
              <EmptyState
                icon={SearchX}
                title={t("common.noMatch")}
                description={t("collections.noMatchHint", { keyword: filter })}
                className="mt-8"
              />
            )}
          </li>
        ) : null}

        {rows.map((row) => {
          if (row.kind === "label") {
            return (
              <li
                key={row.key}
                className="px-2 pb-0.5 pt-2 text-[10px] font-medium tracking-wide text-ink-faint"
              >
                {t(row.labelKey)}
              </li>
            );
          }

          if (row.kind === "folder") {
            const { folder } = row;
            const expanded = !collapsed.has(folder.id);
            // 被拖拽的节点自己、以及它整棵子树里的分类，都是非法目标 → 淡出
            const notAcceptable = drag?.forbidden.has(folder.id) ?? false;

            return (
              <li key={`f-${folder.id}`} role="treeitem" aria-expanded={expanded}>
                {renaming?.kind === "folder" && renaming.id === folder.id ? (
                  <RenameField
                    defaultValue={folder.name}
                    depth={row.depth}
                    onCancel={() => setRenaming(null)}
                    onCommit={(name) => {
                      setRenaming(null);
                      if (name !== folder.name) {
                        void useWorkspaceStore.getState().renameFolder(folder.id, name);
                      }
                    }}
                  />
                ) : (
                  <button
                    type="button"
                    draggable
                    onDragStart={(e) => beginDrag(e, { kind: "folder", id: folder.id })}
                    onDragEnd={endDrag}
                    onDragOver={(e) => dragOver(e, folder.id)}
                    onDragLeave={() => clearDrop(folder.id)}
                    onDrop={(e) => drop(e, folder.id)}
                    onClick={() => toggle(folder.id)}
                    onContextMenu={(e) => openMenu(e, { kind: "folder", folder })}
                    onKeyDown={rowKeys({
                      onMenu: (e) => openMenu(e, { kind: "folder", folder }),
                      onRename: () => setRenaming({ kind: "folder", id: folder.id }),
                      onDelete: () => askDelete({ kind: "folder", id: folder.id }, folder.name),
                    })}
                    style={{ paddingLeft: row.depth * INDENT + 4 }}
                    className={cn(
                      ROW_BASE,
                      "hover:bg-surface-hover",
                      dropTarget === folder.id && DROP_RING,
                      notAcceptable && "opacity-40",
                    )}
                  >
                    <ChevronRight
                      size={12}
                      strokeWidth={2.4}
                      aria-hidden
                      className={cn(
                        "shrink-0 text-ink-faint transition-transform duration-100",
                        expanded && "rotate-90",
                      )}
                    />
                    {expanded ? (
                      <FolderOpen size={13} strokeWidth={1.9} aria-hidden className="shrink-0 text-ink-faint" />
                    ) : (
                      <FolderIcon size={13} strokeWidth={1.9} aria-hidden className="shrink-0 text-ink-faint" />
                    )}
                    <span className="truncate font-medium text-ink">{folder.name}</span>

                    {folder.isTokenSource ? (
                      <span
                        className="shrink-0 rounded-sm bg-brand/12 px-1 text-[10px] font-medium text-brand"
                        title={t("auth.tokenSource")}
                      >
                        {t("auth.tokenSource")}
                      </span>
                    ) : null}

                    <span className="flex-1" />
                    <span
                      aria-label={t("collections.itemCount", { count: row.itemCount })}
                      className="tabular shrink-0 text-[10px] text-ink-faint"
                    >
                      {row.itemCount > 0 ? row.itemCount : ""}
                    </span>
                  </button>
                )}
              </li>
            );
          }

          const { request } = row;
          const active = request.id === activeRequestId;
          // 投放目标是「所在分类」而不是这一行本身 —— 见 DropTarget 的说明
          const ownTarget: DropTarget = request.folderId ?? "root";
          const notAcceptable = drag?.forbidden.has(ownTarget === "root" ? -1 : ownTarget) ?? false;

          return (
            <li key={`r-${request.id}`} role="treeitem" aria-selected={active}>
              {renaming?.kind === "request" && renaming.id === request.id ? (
                <RenameField
                  defaultValue={request.name}
                  depth={row.depth}
                  onCancel={() => setRenaming(null)}
                  onCommit={(name) => {
                    setRenaming(null);
                    if (name !== request.name) {
                      void useWorkspaceStore.getState().renameRequest(request.id, name);
                    }
                  }}
                />
              ) : (
                <button
                  type="button"
                  draggable
                  onDragStart={(e) => beginDrag(e, { kind: "request", id: request.id })}
                  onDragEnd={endDrag}
                  onDragOver={(e) => dragOver(e, ownTarget)}
                  onDragLeave={() => clearDrop(ownTarget)}
                  onDrop={(e) => drop(e, ownTarget)}
                  onClick={() => openInEditor(request)}
                  onContextMenu={(e) => openMenu(e, { kind: "request", request })}
                  onKeyDown={rowKeys({
                    onMenu: (e) => openMenu(e, { kind: "request", request }),
                    onRename: () => setRenaming({ kind: "request", id: request.id }),
                    onDelete: () => askDelete({ kind: "request", id: request.id }, request.name),
                  })}
                  style={{ paddingLeft: row.depth * INDENT + 4 }}
                  className={cn(
                    ROW_BASE,
                    active ? "bg-brand/12 text-ink" : "hover:bg-surface-hover",
                    dropTarget === ownTarget && DROP_RING,
                    notAcceptable && "opacity-40",
                  )}
                >
                  {/* 固定宽度 + 等宽字体：方法名长短不一时，右边名称仍然对齐成一列 */}
                  <span
                    className={cn(
                      "w-[40px] shrink-0 font-mono text-[10px] font-semibold tracking-tight",
                      methodTextClass(request.method),
                    )}
                  >
                    {request.method}
                  </span>
                  <span className={cn("truncate", active ? "text-ink" : "text-ink-muted")}>
                    {request.name}
                  </span>
                </button>
              )}
            </li>
          );
        })}

        {/*
          拖拽时才出现的「根级投放区」。
          常驻的话它会在正常浏览时白占一行；只在拖拽时给，
          既省地方又让「放到最外层」这件事变得可见。
        */}
        {drag ? (
          <li role="treeitem">
            <div
              onDragOver={(e) => dragOver(e, "root")}
              onDragLeave={() => clearDrop("root")}
              onDrop={(e) => drop(e, "root")}
              className={cn(
                "mx-1 mt-1 flex h-7 items-center justify-center gap-1.5 rounded-sm",
                "border border-dashed text-[10px]",
                dropTarget === "root"
                  ? "border-brand bg-brand/12 text-brand"
                  : "border-line text-ink-faint",
              )}
            >
              <CornerDownRight size={11} strokeWidth={2} aria-hidden />
              {t("collections.moveToRoot")}
            </div>
          </li>
        ) : null}
      </ul>

      {menu ? (
        <ContextMenu items={menuItems} x={menu.x} y={menu.y} onClose={() => setMenu(null)} />
      ) : null}

      {moving ? (
        <MoveDialog
          name={moving.name}
          folders={folders}
          currentParentId={currentParentOf(moving.node, folders, requests)}
          forbidden={
            moving.node.kind === "folder"
              ? folderSubtree(folders, moving.node.id)
              : new Set<number>()
          }
          onClose={() => setMoving(null)}
          onConfirm={(parentId) => {
            const store = useWorkspaceStore.getState();
            if (moving.node.kind === "folder") void store.moveFolder(moving.node.id, parentId);
            else void store.moveRequest(moving.node.id, parentId);
          }}
        />
      ) : null}

      {confirmDelete ? (
        <ConfirmDialog
          danger
          title={t(
            confirmDelete.node.kind === "folder"
              ? "collections.deleteFolderTitle"
              : "collections.deleteRequestTitle",
          )}
          description={t(
            confirmDelete.node.kind === "folder"
              ? "collections.deleteFolderDesc"
              : "collections.deleteRequestDesc",
            {
              name: confirmDelete.name,
              folders: confirmDelete.folders,
              requests: confirmDelete.requests,
            },
          )}
          confirmLabel={t("common.delete")}
          onClose={() => setConfirmDelete(null)}
          onConfirm={() => {
            const store = useWorkspaceStore.getState();
            const node = confirmDelete.node;

            // 编辑器里那份内容要是马上就不存在了，就先腾空 ——
            // 否则它会继续挂着一个不存在的 id（绿点还亮着，自动保存也只会静默失败）。
            // 这一步必须放在删除**之前**：`openInEditor(null)` 会先把未落库的
            // 改动写回去，那时目标行还在，改动不会白丢。
            if (editorWillVanish(node, folders, requests)) openInEditor(null);

            if (node.kind === "folder") void store.deleteFolder(node.id);
            else void store.deleteRequest(node.id);
          }}
        />
      ) : null}

      <CreateDialog />
    </>
  );
}

/** 投放目标的视觉：内描边而不是外描边，避免撑开行高导致整棵树跳动 */
const DROP_RING = "bg-brand/12 ring-1 ring-brand ring-inset";

// ---------------------------------------------------------------
// 选中 = 树上的高亮 + 编辑器里的内容
// ---------------------------------------------------------------

/**
 * 选中一个请求，并把它灌进编辑器。
 *
 * 这两件事必须**成对**发生：`activeRequestId` 只管树上的高亮，
 * 编辑器读的是 `store/request` 里的草稿 —— 只设前者的话，
 * 点一下树，右边还是上一个请求的内容。
 *
 * 传 `null` 表示「清空成一张新草稿」，用在「正开着的请求被删掉了」那条路径上。
 */
function openInEditor(request: ApiRequest | null): void {
  useUiStore.getState().setActiveRequestId(request?.id ?? null);
  useRequestStore.getState().openRequest(request);
}

/**
 * 这次删除会不会把编辑器里正开着的那份内容一起带走。
 *
 * 删分类要连带看**整棵子树** —— 请求可能挂在被删分类的更下一层里，
 * 只比「id 是否相等」会漏掉，留下的编辑器会挂着一个不存在的行。
 */
function editorWillVanish(
  node: NodeRef,
  folders: Folder[],
  requests: ApiRequest[],
): boolean {
  const openId = useRequestStore.getState().sourceId;
  // 编辑器里是没保存过的草稿（或历史快照回填），删谁都不影响它
  if (openId === null) return false;
  if (node.kind === "request") return node.id === openId;

  const openRequest = requests.find((r) => r.id === openId);
  // 这一行已经不在存活集合里了 —— 编辑器同样是悬空的，一起清掉
  if (!openRequest) return true;
  return (
    openRequest.folderId !== null &&
    folderSubtree(folders, node.id).has(openRequest.folderId)
  );
}

// ---------------------------------------------------------------
// 新建对话框（由 ui store 驱动，侧栏顶部按钮与右键菜单共用）
// ---------------------------------------------------------------

function CreateDialog() {
  const { t } = useI18n();
  const pendingCreate = useUiStore((s) => s.pendingCreate);
  const close = useUiStore((s) => s.closeCreateDialog);

  if (!pendingCreate) return null;
  const isFolder = pendingCreate.kind === "folder";

  return (
    <PromptDialog
      title={t(isFolder ? "collections.newSubfolder" : "collections.newRequest")}
      placeholder={t(
        isFolder ? "collections.folderNamePlaceholder" : "collections.requestNamePlaceholder",
      )}
      validate={(value) => validateName(value, t)}
      onClose={close}
      onSubmit={(name) => {
        const store = useWorkspaceStore.getState();
        if (isFolder) {
          void store.createFolder(pendingCreate.parentId, name);
        } else {
          void store
            .createRequest(pendingCreate.parentId, name)
            // 建完就打开：新建请求的下一步必然是「在里面写点什么」
            .then((created) => openInEditor(created));
        }
      }}
    />
  );
}

// ---------------------------------------------------------------
// 就地重命名
// ---------------------------------------------------------------

function RenameField({
  defaultValue,
  depth,
  onCommit,
  onCancel,
}: {
  defaultValue: string;
  depth: number;
  onCommit: (name: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(defaultValue);
  /** 防止 Enter 提交之后 blur 又提交一次（Enter 会让输入框失焦） */
  const [done, setDone] = useState(false);

  const commit = () => {
    if (done) return;
    setDone(true);
    const name = value.trim();
    if (name === "" || name === defaultValue) onCancel();
    else onCommit(name);
  };

  return (
    <input
      type="text"
      autoFocus
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onFocus={(e) => e.target.select()}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        } else if (e.key === "Escape") {
          e.preventDefault();
          setDone(true);
          onCancel();
        }
        // 别让方向键/Delete 冒泡到树的快捷键上（否则一边改名字一边删节点）
        e.stopPropagation();
      }}
      spellCheck={false}
      aria-label={defaultValue}
      style={{ paddingLeft: depth * INDENT + 4 }}
      className={cn(
        "h-[26px] w-full rounded-sm border border-brand bg-surface pr-2",
        "text-xs text-ink outline-none",
      )}
    />
  );
}

// ---------------------------------------------------------------
// 拍平
// ---------------------------------------------------------------

type Row =
  | { kind: "folder"; folder: Folder; depth: number; itemCount: number }
  | { kind: "request"; request: ApiRequest; depth: number }
  | { kind: "label"; key: string; labelKey: MessageKey };

function flatten(
  folders: Folder[],
  requests: ApiRequest[],
  collapsed: ReadonlySet<number>,
  keyword: string,
): Row[] {
  const q = keyword.trim().toLowerCase();
  if (q !== "") return flattenFiltered(folders, requests, q);

  const out: Row[] = [];

  const walk = (parentId: number | null, depth: number) => {
    for (const folder of childFolders(folders, parentId)) {
      const subFolders = childFolders(folders, folder.id);
      const own = requestsInFolder(requests, folder.id);
      out.push({
        kind: "folder",
        folder,
        depth,
        itemCount: subFolders.length + own.length,
      });

      if (collapsed.has(folder.id)) continue;

      // 子文件夹在前、请求在后 —— 跟文件管理器的习惯一致
      walk(folder.id, depth + 1);
      for (const r of own) out.push({ kind: "request", request: r, depth: depth + 1 });
    }
  };

  walk(null, 0);

  // 未归档的请求排在整棵树之后。**只有当上面有文件夹时才加分组标题** ——
  // 否则「未归档」会贴在空列表里，看起来像个莫名其妙的分类。
  const unfiled = requestsInFolder(requests, null);
  if (unfiled.length > 0 && folders.length > 0) {
    out.push({ kind: "label", key: "label-unfiled", labelKey: "collections.unfiled" });
  }
  for (const r of unfiled) {
    out.push({ kind: "request", request: r, depth: 0 });
  }

  return out;
}

/**
 * 筛选视图。
 *
 * 命中规则：
 * - 请求命中它的**名字**或**地址**（地址往往比名字更好认，「是我调过的那个 v2 接口」）
 * - 分类只命中名字，但它的**祖先链会被一起补上** ——
 *   否则结果里会出现一串没有父级的孤儿行，用户根本不知道它在哪一层
 *
 * 筛选时**忽略折叠状态**：命中的东西必须看得见。
 * 「搜到了，但藏在收起的分类里」等于没搜到。
 */
function flattenFiltered(
  folders: Folder[],
  requests: ApiRequest[],
  q: string,
): Row[] {
  const byId = new Map(folders.map((f) => [f.id, f] as const));

  const hitFolders = new Set<number>();
  for (const f of folders) {
    if (f.name.toLowerCase().includes(q)) hitFolders.add(f.id);
  }
  // 补祖先。循环体本身在往集合里加元素，所以库里即使有环（导入来的脏数据）
  // 也会停下来 —— 与后端 `UNION` 而非 `UNION ALL`、`folderSubtree` 的 `Set` 同理
  for (const id of [...hitFolders]) {
    let parent = byId.get(id)?.parentId ?? null;
    while (parent !== null && !hitFolders.has(parent)) {
      hitFolders.add(parent);
      parent = byId.get(parent)?.parentId ?? null;
    }
  }

  const hitRequests = new Set<number>();
  for (const r of requests) {
    if (r.name.toLowerCase().includes(q) || r.url.toLowerCase().includes(q)) {
      hitRequests.add(r.id);
    }
  }

  const out: Row[] = [];

  const walk = (parentId: number | null, depth: number) => {
    for (const folder of childFolders(folders, parentId)) {
      if (!hitFolders.has(folder.id)) continue;
      const subFolders = childFolders(folders, folder.id).filter((f) =>
        hitFolders.has(f.id),
      );
      const own = requestsInFolder(requests, folder.id).filter((r) =>
        hitRequests.has(r.id),
      );
      // 计数用「命中的子项数」而不是真实总数：列表里只显示命中的那几个，
      // 显示总数会得到「3 项」但下面只有 1 行
      out.push({
        kind: "folder",
        folder,
        depth,
        itemCount: subFolders.length + own.length,
      });
      walk(folder.id, depth + 1);
      for (const r of own) out.push({ kind: "request", request: r, depth: depth + 1 });
    }
  };
  walk(null, 0);

  const unfiled = requestsInFolder(requests, null).filter((r) => hitRequests.has(r.id));
  if (unfiled.length > 0 && out.length > 0) {
    out.push({ kind: "label", key: "label-unfiled", labelKey: "collections.unfiled" });
  }
  for (const r of unfiled) out.push({ kind: "request", request: r, depth: 0 });

  return out;
}

/** 某个节点当前挂在哪个父节点下（移动对话框用来高亮当前项） */
function currentParentOf(
  node: NodeRef,
  folders: Folder[],
  requests: ApiRequest[],
): number | null {
  if (node.kind === "folder") return folders.find((f) => f.id === node.id)?.parentId ?? null;
  return requests.find((r) => r.id === node.id)?.folderId ?? null;
}

/** 行内的键盘快捷键：Shift+F10 / 菜单键打开菜单，F2 重命名，Delete 删除 */
function rowKeys(handlers: {
  onMenu: (e: React.KeyboardEvent) => void;
  onRename: () => void;
  onDelete: () => void;
}) {
  return (e: React.KeyboardEvent) => {
    // Shift+F10 与「菜单键」是各平台约定的「右键菜单」键盘等价物。
    // 有了它，整个集合树的管理功能都不依赖鼠标 —— 拖拽反而只是快捷方式。
    if ((e.shiftKey && e.key === "F10") || e.key === "ContextMenu") {
      handlers.onMenu(e);
      return;
    }
    if (e.key === "F2") {
      e.preventDefault();
      handlers.onRename();
      return;
    }
    if (e.key === "Delete") {
      e.preventDefault();
      handlers.onDelete();
    }
  };
}

const ROW_BASE =
  "flex h-[26px] w-full items-center gap-1.5 rounded-sm pr-1.5 text-left text-xs " +
  "transition-colors duration-75 select-none";
