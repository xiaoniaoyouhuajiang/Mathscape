import { markdownRenderer } from "../reader/markdown";

// Cards read the authorized excerpt; images and navigation stay in the desktop reader.
const renderer = markdownRenderer();
renderer.renderer.rules.link_open = () => "<span>";
renderer.renderer.rules.link_close = () => "</span>";

export function problemCardMarkdown(source: string): string {
  return renderer.render(source);
}
