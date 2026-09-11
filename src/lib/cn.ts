/**
 * 类名拼接。等价于 clsx 的最小子集 ——
 * 我们不需要 tailwind-merge 的冲突消解（组件内部不做条件覆盖
 * 同一属性类的操作），所以不引入依赖。
 */
export type ClassValue =
  | string
  | number
  | null
  | undefined
  | false
  | readonly ClassValue[];

function walk(value: ClassValue, out: string[]): void {
  if (!value) return;
  if (Array.isArray(value)) {
    for (const item of value) walk(item, out);
    return;
  }
  out.push(String(value));
}

export function cn(...inputs: readonly ClassValue[]): string {
  const out: string[] = [];
  for (const input of inputs) walk(input, out);
  return out.join(" ");
}
