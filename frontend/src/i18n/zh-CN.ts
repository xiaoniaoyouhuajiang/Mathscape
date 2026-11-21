/**
 * 界面文案的源语言字典。新增文案先加在这里，再在 en.ts 补对应译文；
 * 键名按模块分段（`theme.*`、`error.*`），`{name}` 是插值占位符。
 */
export const zhCN = {
  "theme.obsidian": "深空星图 · Obsidian",
  "theme.parchment": "暖宣手稿 · Parchment",
  "theme.slate": "青岩静夜 · Slate",
  "theme.toggleTitle": "当前主题：{label}；点击切换",
  "theme.toggleAriaLabel": "切换主题，当前：{label}",
  "error.unknown": "发生了未知错误",
  "review.label_lang_missing": "共享对象缺少中文或英文名称",
  "review.possible_duplicate_name": "名称或别名与已有的同类对象相同，可能重复",
  "review.duplicate_mathlib_decl": "mathlib 名称已被同类对象使用，可能重复",
} satisfies Record<string, string>;

export type MessageKey = keyof typeof zhCN;
export type Messages = Record<MessageKey, string>;
