/**
 * 界面文案的源语言字典。新增文案先加在这里，再在 en.ts 补对应译文；
 * 键名按模块分段（`theme.*`、`reader.*`、`error.*`），`{name}` 是插值占位符。
 */
export const zhCN = {
  "theme.obsidian": "深空星图 · Obsidian",
  "theme.parchment": "暖宣手稿 · Parchment",
  "theme.slate": "青岩静夜 · Slate",
  "theme.toggleTitle": "当前主题：{label}；点击切换",
  "theme.toggleAriaLabel": "切换主题，当前：{label}",
  "reader.canonical.continue": "在规范解析正文中继续阅读",
  "reader.canonical.continueTitle": "切换到按页序拼接的完整正文并停在当前这一块，可以继续向前或向后阅读",
  "reader.canonical.continueFromHere": "在规范解析正文中从这里继续阅读",
  "reader.canonical.noPositionNotice": "这一处在规范解析正文中没有对应的可读位置，已保留在解析快照中。",
  "reader.canonical.unreadableNotice": "规范解析正文暂时打不开（有分段读取失败），已保留在解析快照中。",
  "reader.canonical.pageFromOtherSnapshot": "这一块所在的 PDF 第 {page} 页在规范解析正文中由另一个解析快照显示，已定位到该页开头。",
  "reader.canonical.viewInSnapshot": "在原快照中查看这一块",
  "reader.snapshotTitle": "{title} · 解析快照",
  "reader.pdfPage": "PDF 第 {page} 页",
  "reader.notice.dismiss": "知道了",
  "reader.boundary.readUp": "继续向上阅读",
  "reader.boundary.readDown": "继续向下阅读",
  "reader.boundary.buttonTitle": "进入按页序拼接的规范解析正文，停在这一块",
  "reader.boundary.startsAtPage": "本快照从 PDF 第 {page} 页开始。要接着往前读，请进入规范解析正文。",
  "reader.boundary.startUnknown": "本快照只包含一次解析的页。要接着往前读，请进入规范解析正文。",
  "reader.boundary.endsAtPage": "本快照到 PDF 第 {page} 页为止。要接着往后读，请进入规范解析正文。",
  "reader.boundary.endUnknown": "本快照只包含一次解析的页。要接着往后读，请进入规范解析正文。",
  "reader.return.back": "↩ 回到出发处 · {label}",
  "reader.return.backTitle": "滚回刚才定位到的那一块",
  "reader.return.dismissTitle": "不再显示",
  "reader.return.dismissAriaLabel": "不再显示回到出发处",
  "reader.blockLabelFallback": "定位块",
  "error.unknown": "发生了未知错误",
} satisfies Record<string, string>;

export type MessageKey = keyof typeof zhCN;
export type Messages = Record<MessageKey, string>;
