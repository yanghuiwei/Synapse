/* ============================================================
   名称校验 —— 与后端 `commands::validate` 对齐
   ------------------------------------------------------------
   两处都做不是重复劳动：后端那份是**权威**（防住绕过 UI 直接调 IPC
   的路径），前端这份是为了**在输入时就把确认按钮禁掉** ——
   让用户敲完一长串名字再被后端拒掉，体验上等于白干一次。

   原先这份逻辑在 `CollectionTree` 和 `EnvManager` 里各抄了一遍，
   加「保存到分类」的对话框时就该变成第三份了 —— 所以收到这里。
   ============================================================ */

import type { TranslateFn } from "@/lib/i18n";

/** 与后端 `clean_name` 对齐的名字长度上限 */
export const MAX_NAME_LEN = 256;

/**
 * 校验一个名字。
 *
 * @param value 原始输入（内部会先 trim）
 * @returns `null` 表示通过；否则是给用户看的错误文案
 */
export function validateName(value: string, t: TranslateFn): string | null {
  const trimmed = value.trim();
  if (trimmed === "") return t("collections.nameEmpty");
  // 用展开而不是 `.length`：一个汉字算 1 个字符，与后端 `chars().count()` 对齐
  if ([...trimmed].length > MAX_NAME_LEN) {
    return t("collections.nameTooLong", { max: MAX_NAME_LEN });
  }
  return null;
}
