import { mockIPC } from "@tauri-apps/api/mocks";
import { ArtifactReader } from "../src/reader/reader";
import { markdownRenderer } from "../src/reader/markdown";
import { resolveProductPath, readRange, type ContentInfo } from "../src/reader/content";

// Tiny reproducible PDF: two pages and one outline entry, no copyrighted assets.
function samplePdf(): Uint8Array {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 5 0 R >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 6 0 R >>",
    "<< /Length 25 >>\nstream\n1 0 0 rg 0 0 100 100 re f\n\nendstream",
    "<< /Length 25 >>\nstream\n0 0 1 rg 0 0 200 200 re f\n\nendstream",
    "<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 1 >>",
    "<< /Title (Second page) /Parent 7 0 R /Dest [4 0 R /Fit] >>",
  ];
  // Padding ensures PDF.js must request ranges rather than receive the whole file initially.
  let data = "%PDF-1.4\n%" + "padding".repeat(40000) + "\n";
  const offsets = [0];
  objects.forEach((body, i) => { offsets.push(data.length); data += `${i + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = data.length;
  data += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  offsets.slice(1).forEach(offset => { data += `${String(offset).padStart(10, "0")} 00000 n \n`; });
  data += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return new TextEncoder().encode(data);
}

const markdown = String.raw`# Math reader

Inline $x^2$ and \(a_n\to a\).

$$
\frac{1}{n}\sum_{k=1}^n a_k

\longrightarrow a
$$

\[
\omega(s)=(s_1,\ldots,s_k)
\]

![Diagram](images/plot.png)
![Unsafe](https://example.com/tracker.png)

<script>window.untrusted = true</script>

~~~tex
$code_not_math$
~~~
`;
const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOZkAAAAASUVORK5CYII="), c => c.charCodeAt(0));
const files: Record<string, { media: string; bytes: Uint8Array }> = {
  pdf: { media: "application/pdf", bytes: samplePdf() },
  md: { media: "text/markdown", bytes: new TextEncoder().encode(markdown) },
  png: { media: "image/png", bytes: png },
  json: { media: "application/json", bytes: new TextEncoder().encode('{"page":1}') },
  bad: { media: "application/pdf", bytes: new TextEncoder().encode("not a PDF") },
};
const reads: { id: string; offset: number; length: number }[] = [];
mockIPC((command, payload) => {
  const args = payload as { artifactId: string; offset: number; length: number };
  const file = files[args.artifactId];
  if (!file) throw new Error("Artifact 不存在");
  if (command === "get_artifact_content_info") return {
    artifact_id: args.artifactId, media_type: file.media, byte_size: file.bytes.length,
    contexts: args.artifactId === "md" ? [{ conversion_id: "conversion", paths: [
      { path: "book/book.md", artifact_id: "md" },
      { path: "book/images/plot.png", artifact_id: "png" },
    ] }] : [],
  } satisfies ContentInfo;
  if (command === "read_artifact_bytes") {
    reads.push({ id: args.artifactId, offset: args.offset, length: args.length });
    return file.bytes.slice(args.offset, args.offset + args.length).buffer;
  }
  throw new Error("Unexpected IPC: " + command);
});

const reader = new ArtifactReader(document.querySelector<HTMLElement>("#reader")!);
Object.assign(window, {
  harness: { reader, reads, resolveProductPath, readRange, render: (source: string) => markdownRenderer().render(source) },
});
