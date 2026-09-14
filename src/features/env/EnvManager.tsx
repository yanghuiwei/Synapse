import { Check, Globe, ListTree, LoaderCircle, Pencil, Trash, TriangleAlert } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { Environment } from "@/lib/types";
import { useEnvStore } from "@/store/env";
import { useUiStore } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { Button, IconButton } from "@/features/ui/Button";
import { ConfirmDialog, PromptDialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { EnvVarDialog } from "./EnvVarDialog";

/** 与 `commands::validate::MAX_NAME_LEN` 一致 */
const MAX_NAME_LEN = 256;

/**
 * 环境板块（侧栏）。
 *
 * ## 为什么变量表在对话框里，而不是铺在侧栏里
 *
 * 侧栏最窄 180px、默认 260px，而变量表至少需要「启用 / 键 / 值 / 密钥 / 删除」
 * 五列。硬塞进去的结果是每列都窄到看不清值。列表留在侧栏（它只需要一列名字），
 * 编辑进对话框（它有 620px）。
 *
 * ## 为什么「激活」是一个显式的动作
 *
 * 「当前环境」的唯一真源是 `environment.is_active` 列。界面这里点一下 →
 * 走后端命令 → 整块重拉，**不做**乐观更新：两处状态一旦分叉，
 * 发送路径读的是后端那一份、界面上显示另一份，而那种 bug 从界面上看不出来。
 */
export function EnvManager() {
  const { t } = useI18n();

  const status = useEnvStore((s) => s.status);
  const error = useEnvStore((s) => s.error);
  const environments = useEnvStore((s) => s.environments);
  const active = useEnvStore((s) => s.active);
  const load = useEnvStore((s) => s.load);
  const setActive = useEnvStore((s) => s.setActive);

  const envDialog = useUiStore((s) => s.envDialog);
  const openEnvDialog = useUiStore((s) => s.openEnvDialog);
  const closeEnvDialog = useUiStore((s) => s.closeEnvDialog);

  /**
   * ⚠️ 依赖必须是 `workspaceId`，不能只依赖「挂载一次」。
   *
   * 首挂时 `workspace.load()` 还在路上，`load()` 拿到的 workspace 是 null
   * 就直接返回了 —— 那样界面会**永远**停在空态（而且没有报错，
   * 只有无头截图才抓得到）。
   */
  const workspaceId = useWorkspaceStore((s) => s.workspace?.id);
  useEffect(() => {
    if (workspaceId === undefined) return;
    void load();
  }, [workspaceId, load]);

  const [renameTarget, setRenameTarget] = useState<Environment | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Environment | null>(null);

  if (status === "idle" || status === "loading") {
    return (
      <div className="mt-8 flex items-center justify-center gap-2 px-6 text-xs text-ink-faint">
        <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
        <span>{t("common.loading")}</span>
      </div>
    );
  }

  if (status === "error") {
    return (
      <EmptyState
        icon={TriangleAlert}
        title={t("env.loadFailed")}
        // 文案来自后端错误码，不在这里写死（见 lib/ipc.ts 的契约）
        description={error?.localized}
        action={
          <Button size="sm" variant="secondary" onClick={() => void load()}>
            {t("common.retry")}
          </Button>
        }
        className="mt-8"
      />
    );
  }

  const editTarget =
    envDialog?.kind === "edit"
      ? (environments.find((e) => e.id === envDialog.id) ?? null)
      : null;

  return (
    <div className="pt-0.5">
      {environments.length === 0 ? (
        <EmptyState
          icon={Globe}
          title={t("env.empty")}
          description={t("env.emptyHint")}
          className="mt-6"
        />
      ) : (
        <ul className="space-y-0.5">
          {/* 「不使用环境」是一等选项，不是「什么也不选」——
              它是常态（本地调试时常常不需要），所以要能被明确地选中 */}
          <li>
            <Row
              label={t("env.none")}
              selected={active === null}
              onActivate={() => void setActive(null)}
            />
          </li>

          {environments.map((env) => (
            <li key={env.id} className="group">
              <Row
                label={env.name}
                selected={active?.id === env.id}
                onActivate={() => void setActive(env.id)}
                actions={
                  <>
                    <IconButton
                      size="sm"
                      label={t("env.variable")}
                      onClick={() => openEnvDialog({ kind: "edit", id: env.id })}
                    >
                      <ListTree size={12} strokeWidth={2} />
                    </IconButton>
                    <IconButton
                      size="sm"
                      label={t("common.rename")}
                      onClick={() => setRenameTarget(env)}
                    >
                      <Pencil size={12} strokeWidth={2} />
                    </IconButton>
                    <IconButton
                      size="sm"
                      label={t("common.delete")}
                      variant="danger"
                      onClick={() => setDeleteTarget(env)}
                    >
                      <Trash size={12} strokeWidth={2} />
                    </IconButton>
                  </>
                }
              />
            </li>
          ))}
        </ul>
      )}

      {envDialog?.kind === "new" ? (
        <NewEnvDialog onClose={closeEnvDialog} />
      ) : editTarget ? (
        <EnvVarDialog environment={editTarget} onClose={closeEnvDialog} />
      ) : null}

      {renameTarget ? (
        <PromptDialog
          title={t("env.renameTitle")}
          defaultValue={renameTarget.name}
          placeholder={t("env.name")}
          validate={(value) =>
            value.length > MAX_NAME_LEN
              ? t("collections.nameTooLong", { max: MAX_NAME_LEN })
              : null
          }
          onSubmit={(name) => void useEnvStore.getState().rename(renameTarget.id, name)}
          onClose={() => setRenameTarget(null)}
        />
      ) : null}

      {deleteTarget ? (
        <ConfirmDialog
          title={t("env.deleteTitle")}
          description={t("env.deleteDesc", { name: deleteTarget.name })}
          confirmLabel={t("common.delete")}
          onConfirm={() => void useEnvStore.getState().remove(deleteTarget.id)}
          onClose={() => setDeleteTarget(null)}
        />
      ) : null}
    </div>
  );
}

/**
 * 一行环境。
 *
 * 激活按钮**常驻可见**（只做淡色，不做 `opacity-0 group-hover:opacity-100`）：
 * 藏起来的按钮等于不存在，而「切换环境」是这个板块最高频的动作。
 * 只有编辑/重命名/删除那三个次要动作才在悬停时淡入。
 */
function Row({
  label,
  selected,
  onActivate,
  actions,
}: {
  label: string;
  selected: boolean;
  onActivate: () => void;
  actions?: ReactNode;
}) {
  const { t } = useI18n();
  return (
    <div
      className={cn(
        "flex h-7 items-center gap-1 rounded-sm px-1",
        selected ? "bg-brand/12" : "hover:bg-surface-hover",
      )}
    >
      <button
        type="button"
        onClick={onActivate}
        title={selected ? t("env.active") : t("env.activate")}
        aria-current={selected ? "true" : undefined}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
      >
        <span className="flex h-4 w-4 shrink-0 items-center justify-center">
          {selected ? (
            <Check size={12} strokeWidth={2.6} aria-hidden className="text-brand" />
          ) : null}
        </span>
        <span
          className={cn(
            "truncate text-xs",
            selected ? "text-brand" : "text-ink",
          )}
        >
          {label}
        </span>
      </button>

      {actions ? (
        <span className="flex shrink-0 items-center gap-px opacity-45 transition-opacity duration-100 group-hover:opacity-100 focus-within:opacity-100">
          {actions}
        </span>
      ) : null}
    </div>
  );
}

function NewEnvDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const create = useEnvStore((s) => s.create);
  const openEnvDialog = useUiStore((s) => s.openEnvDialog);

  return (
    <PromptDialog
      title={t("env.new")}
      placeholder={t("env.name")}
      validate={(value) =>
        value.length > MAX_NAME_LEN
          ? t("collections.nameTooLong", { max: MAX_NAME_LEN })
          : null
      }
      onSubmit={(name) => {
        // 建完**立刻**打开变量表：一个没有变量的环境没有任何用处，
        // 让用户再找一次「编辑」是白白多一步
        void create(name).then((created) => {
          if (created) openEnvDialog({ kind: "edit", id: created.id });
          else onClose();
        });
      }}
      onClose={onClose}
    />
  );
}
