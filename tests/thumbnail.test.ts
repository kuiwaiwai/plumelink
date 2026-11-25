import { expect, it } from "vitest";
import { crc32, deflateSync } from "node:zlib";
import { thumbnail } from "../shared/thumbnail";

function bmp(width = 1, height = 1) {
  const bytes = Buffer.alloc(54 + width * height * 4);
  bytes.write("BM");
  bytes.writeUInt32LE(bytes.length, 2);
  bytes.writeUInt32LE(54, 10);
  bytes.writeUInt32LE(40, 14);
  bytes.writeInt32LE(width, 18);
  bytes.writeInt32LE(height, 22);
  bytes.writeUInt16LE(1, 26);
  bytes.writeUInt16LE(32, 28);
  return bytes;
}
const url = (bytes: Buffer) => "data:image/bmp;base64," + bytes.toString("base64");

it("accepts bounded native bitmaps without remote image requests", () => {
  for (const size of [1, 32, 64]) {
    const icon = url(bmp(size, size));
    expect(thumbnail(icon)).toBe(icon);
  }
  for (const value of [null, {}, "https://example.com/icon.bmp", "data:image/svg+xml,<svg/>", "data:image/bmp;base64,?", url(bmp(65, 65))])
    expect(thumbnail(value)).toBeUndefined();
});
it("rejects truncated data, oversized dimensions and unsupported bitmap layouts", () => {
  for (const [offset, value] of [[2, 0], [10, 55], [14, 124], [18, 0], [22, 65], [30, 1]]) {
    const bytes = bmp();
    bytes.writeUInt32LE(value, offset);
    expect(thumbnail(url(bytes))).toBeUndefined();
  }
  const bytes = bmp();
  bytes.writeUInt16LE(24, 28);
  expect(thumbnail(url(bytes))).toBeUndefined();
  expect(thumbnail(url(bmp().subarray(0, 56)))).toBeUndefined();
  const legacy=url(bmp(128,128));
  expect(thumbnail(legacy)).toBeUndefined();
  expect(thumbnail(legacy,128)).toBe(legacy);
  expect(thumbnail(url(bmp(129,129)),128)).toBeUndefined();
});

it("accepts transparent PNGs while bounding their format and dimensions", () => {
  const chunk = (type: string, data: Buffer) => {
    const payload = Buffer.concat([Buffer.from(type), data]);
    const size = Buffer.alloc(4), checksum = Buffer.alloc(4);
    size.writeUInt32BE(data.length); checksum.writeUInt32BE(crc32(payload));
    return Buffer.concat([size, payload, checksum]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(1); header.writeUInt32BE(1, 4); header[8] = 8; header[9] = 6;
  const bytes = Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk("IHDR", header),
    chunk("IDAT", deflateSync(Buffer.from([0,255,0,0,128]))), chunk("IEND", Buffer.alloc(0))]);
  const encode = (data: Buffer) => "data:image/png;base64," + data.toString("base64");
  expect(thumbnail(encode(bytes))).toBe(encode(bytes));
  for (const [offset, value] of [[16,65], [20,0], [12,0]]) {
    const invalid = Buffer.from(bytes); invalid.writeUInt32BE(value,offset);
    expect(thumbnail(encode(invalid))).toBeUndefined();
  }
  expect(thumbnail(encode(bytes.subarray(0,-1)))).toBeUndefined();
});
