// Generates a 1024x1024 PNG icon source with no external deps.
// A rounded-square gradient with an "M" glyph drawn as filled rectangles.
import zlib from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";

const SIZE = 1024;

function crc32(buf) {
  let c;
  const table = [];
  for (let n = 0; n < 256; n++) {
    c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) crc = table[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const body = Buffer.concat([typeBuf, data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([len, body, crc]);
}

// Build RGBA pixels.
const px = Buffer.alloc(SIZE * SIZE * 4);
const radius = 180;
function inRounded(x, y) {
  const r = radius;
  const minX = r, maxX = SIZE - r, minY = r, maxY = SIZE - r;
  if (x >= minX && x <= maxX) return true;
  if (y >= minY && y <= maxY) return true;
  // corners
  const cx = x < minX ? minX : maxX;
  const cy = y < minY ? minY : maxY;
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
}

// "M" glyph via three thick strokes.
function inGlyph(x, y) {
  const left = 300, right = 724, top = 320, bottom = 704, stroke = 96;
  // left vertical
  if (x >= left && x <= left + stroke && y >= top && y <= bottom) return true;
  // right vertical
  if (x >= right - stroke && x <= right && y >= top && y <= bottom) return true;
  // diagonals down to center
  const midX = (left + right) / 2;
  const t = (y - top) / (bottom - top);
  if (y >= top && y <= (top + bottom) / 2 + 20) {
    const lx = left + stroke / 2 + t * (midX - left - stroke / 2) * 2;
    const rx = right - stroke / 2 - t * (right - stroke / 2 - midX) * 2;
    if (Math.abs(x - lx) <= stroke / 2) return true;
    if (Math.abs(x - rx) <= stroke / 2) return true;
  }
  return false;
}

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    const i = (y * SIZE + x) * 4;
    if (!inRounded(x, y)) {
      px[i] = 0; px[i + 1] = 0; px[i + 2] = 0; px[i + 3] = 0;
      continue;
    }
    // diagonal gradient: indigo -> violet
    const t = (x + y) / (SIZE * 2);
    const r = Math.round(79 + t * (124 - 79));
    const g = Math.round(70 + t * (58 - 70));
    const b = Math.round(229 + t * (237 - 229));
    if (inGlyph(x, y)) {
      px[i] = 255; px[i + 1] = 255; px[i + 2] = 255; px[i + 3] = 255;
    } else {
      px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = 255;
    }
  }
}

// Add filter byte (0) per scanline.
const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y++) {
  raw[y * (SIZE * 4 + 1)] = 0;
  px.copy(raw, y * (SIZE * 4 + 1) + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // color type RGBA
ihdr[10] = 0;
ihdr[11] = 0;
ihdr[12] = 0;

const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const png = Buffer.concat([
  sig,
  chunk("IHDR", ihdr),
  chunk("IDAT", zlib.deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

mkdirSync(new URL("../src-tauri/icons/", import.meta.url), { recursive: true });
const out = new URL("../src-tauri/icons/source.png", import.meta.url);
writeFileSync(out, png);
console.log("Wrote", out.pathname, png.length, "bytes");
