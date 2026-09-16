import { PDFDataRangeTransport } from "pdfjs-dist/legacy/build/pdf.mjs";

/** IPC reads cannot be cancelled, but their late results must not reach PDF.js. */
export class ArtifactPdfTransport extends PDFDataRangeTransport {
  stopped = false;

  constructor(
    length: number,
    initial: Uint8Array,
    private readonly read: (begin: number, length: number) => Promise<Uint8Array>,
    private readonly fail: (error: Error) => void,
  ) {
    super(length, initial, true);
  }

  override abort(): void {
    this.stopped = true;
  }

  override requestDataRange(begin: number, end: number): void {
    if (this.stopped) return;
    void this.deliver(begin, end);
  }

  private async deliver(begin: number, end: number): Promise<void> {
    let phase = "读取字节";
    try {
      const bytes = await this.read(begin, end - begin);
      if (this.stopped) return;
      phase = "投递 PDF.js";
      this.onDataRange(begin, bytes);
    } catch (cause) {
      if (this.stopped) return;
      this.abort(); // Stop all other callbacks before reporting the first failure.
      this.fail(new Error(`PDF 读取失败（${phase}，字节 [${begin}, ${end})）：${String(cause)}`));
    }
  }
}
