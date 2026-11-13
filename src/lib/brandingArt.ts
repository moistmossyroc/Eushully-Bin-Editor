/**
 * Canvas renderers for fanpatch branding art (SYS4 title badge + boot splash),
 * styled after the official Eushully assets we matched in-game (docs/engine/
 * age-sys.md §4–5). Each renderer draws into a caller-supplied canvas sized to
 * the game profile; the dialog exports it as a PNG data URL for the backend.
 */

/** Gold text with a dark outline, the look of official append banners. */
function drawGoldText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  fontPx: number,
  fontFamily: string,
  align: CanvasTextAlign,
): void {
  ctx.font = `bold ${fontPx}px ${fontFamily}`;
  ctx.textAlign = align;
  ctx.textBaseline = "middle";
  ctx.lineJoin = "round";
  ctx.lineWidth = Math.max(2, fontPx / 6);
  ctx.strokeStyle = "rgb(40, 22, 6)";
  ctx.strokeText(text, x, y);
  const gold = ctx.createLinearGradient(0, y - fontPx / 2, 0, y + fontPx / 2);
  gold.addColorStop(0, "rgb(255, 244, 175)");
  gold.addColorStop(1, "rgb(212, 150, 40)");
  ctx.fillStyle = gold;
  ctx.fillText(text, x, y);
}

/** Banner-array style (Madou/LaDEA/Sankai): transparent strip, gold text left-aligned. */
export function renderBannerBadge(canvas: HTMLCanvasElement, text: string): void {
  const ctx = canvas.getContext("2d")!;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  const fontPx = Math.round(canvas.height * 0.68);
  drawGoldText(ctx, text, 6, canvas.height / 2 + 1, fontPx, "'MS Gothic', 'Noto Sans JP', monospace", "left");
}

/** Tile style (Tenmei AP tiles): beige plate with gold lettering + a label strip. */
export function renderTileBadge(canvas: HTMLCanvasElement, plateText: string, labelText: string): void {
  const ctx = canvas.getContext("2d")!;
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);
  const plateHeight = Math.round(height * 0.62);
  // Plate: dark rim, beige body (matches the official AP tiles).
  ctx.fillStyle = "rgb(60, 40, 20)";
  ctx.beginPath();
  ctx.roundRect(0, 0, width, plateHeight, 3);
  ctx.fill();
  const body = ctx.createLinearGradient(0, 2, 0, plateHeight - 2);
  body.addColorStop(0, "rgb(246, 232, 196)");
  body.addColorStop(1, "rgb(214, 190, 140)");
  ctx.fillStyle = body;
  ctx.beginPath();
  ctx.roundRect(2, 2, width - 4, plateHeight - 4, 2);
  ctx.fill();
  drawGoldText(ctx, plateText, width / 2, plateHeight / 2 + 1, Math.round(plateHeight * 0.6), "Arial, sans-serif", "center");
  // Label strip (the "INSTALL済" line).
  ctx.fillStyle = "rgb(12, 10, 10)";
  ctx.fillRect(0, plateHeight + 1, width, height - plateHeight - 1);
  ctx.font = `bold ${Math.max(8, Math.round((height - plateHeight) * 0.75))}px Tahoma, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = "rgb(205, 205, 205)";
  ctx.fillText(labelText, width / 2, plateHeight + (height - plateHeight) / 2 + 1);
}

export interface SplashText {
  header: string;
  subtitle: string;
  bodyLines: string[];
}

/** Boot splash in the style of the official warning splash: black, gold header,
 *  cream body. With `image`, the art sits left (square, framed) and text right. */
export function renderSplash(canvas: HTMLCanvasElement, text: SplashText, image: CanvasImageSource | null): void {
  const ctx = canvas.getContext("2d")!;
  const { width, height } = canvas;
  ctx.fillStyle = "black";
  ctx.fillRect(0, 0, width, height);
  const scale = height / 900;
  let textLeft = 0;
  let textWidth = width;
  if (image) {
    const side = Math.round(760 * scale);
    const x = Math.round(90 * scale);
    const y = Math.round((height - side) / 2);
    ctx.drawImage(image, x, y, side, side);
    ctx.strokeStyle = "rgb(214, 160, 50)";
    ctx.lineWidth = Math.max(2, 4 * scale);
    ctx.strokeRect(x - 2, y - 2, side + 4, side + 4);
    textLeft = x + side + Math.round(50 * scale);
    textWidth = width - textLeft - Math.round(40 * scale);
  }
  const centerX = textLeft + textWidth / 2;
  const headerY = (image ? 270 : 125) * scale;
  drawGoldText(ctx, text.header, centerX, headerY, Math.round(40 * scale), "'Times New Roman', serif", "center");
  // Ornament rules either side of the header.
  const headerWidth = ctx.measureText(text.header).width;
  ctx.strokeStyle = "rgb(214, 160, 50)";
  ctx.lineWidth = Math.max(1, 2 * scale);
  for (const direction of [-1, 1]) {
    const start = centerX + direction * (headerWidth / 2 + 16 * scale);
    ctx.beginPath();
    ctx.moveTo(start, headerY);
    ctx.lineTo(start + direction * 70 * scale, headerY);
    ctx.stroke();
  }
  drawGoldText(ctx, text.subtitle, centerX, headerY + 85 * scale, Math.round(30 * scale), "'Times New Roman', serif", "center");
  const bodyPx = Math.round((image ? 19 : 24) * scale);
  ctx.font = `bold ${bodyPx}px 'Times New Roman', serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  let y = headerY + 175 * scale;
  for (const line of text.bodyLines) {
    if (line) {
      ctx.shadowColor = "rgba(120, 70, 10, 0.8)";
      ctx.shadowBlur = 4 * scale;
      ctx.fillStyle = "rgb(250, 238, 210)";
      ctx.fillText(line, centerX, y, textWidth);
      ctx.shadowBlur = 0;
    }
    y += bodyPx * 1.6;
  }
}

/** Default disclaimer body for a fan translation splash. */
export const DEFAULT_SPLASH_LINES = [
  "This is an unofficial, non-commercial fan translation.",
  "It is not affiliated with or endorsed by Eushully.",
  "",
  "Please support the official release.",
];
