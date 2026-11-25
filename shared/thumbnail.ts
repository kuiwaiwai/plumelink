export function thumbnail(value: unknown, maxSize: 64 | 128 = 64): string | undefined {
  if (typeof value !== "string" || value.length > (maxSize === 128 ? 90000 : 24000)) return;
  const prefix = value.startsWith("data:image/png;base64,") ? "data:image/png;base64," : "data:image/bmp;base64,";
  if (!value.startsWith(prefix)) return;
  const encoded = value.slice(prefix.length);
  if (!encoded.length || encoded.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(encoded)) return;
  try {
    const raw = atob(encoded);
    const data = new DataView(Uint8Array.from(raw, char => char.charCodeAt(0)).buffer);
    if (prefix === "data:image/png;base64,") {
      if (raw.length < 57 || data.getUint32(0) !== 0x89504e47 || data.getUint32(4) !== 0x0d0a1a0a ||
        data.getUint32(8) !== 13 || data.getUint32(12) !== 0x49484452 ||
        data.getUint32(16) < 1 || data.getUint32(16) > maxSize || data.getUint32(20) < 1 || data.getUint32(20) > maxSize ||
        data.getUint8(24) !== 8 || data.getUint8(25) !== 6 || data.getUint8(26) !== 0 || data.getUint8(27) !== 0 || data.getUint8(28) !== 0 ||
        data.getUint32(raw.length - 12) !== 0 || data.getUint32(raw.length - 8) !== 0x49454e44) return;
      return value;
    }
    if (raw.length < 58) return;
    const width = data.getInt32(18, true), height = data.getInt32(22, true);
    if (data.getUint16(0, true) !== 0x4d42 || data.getUint32(2, true) !== raw.length ||
      data.getUint32(10, true) !== 54 || data.getUint32(14, true) !== 40 ||
      width < 1 || width > maxSize || height < 1 || height > maxSize ||
      data.getUint16(26, true) !== 1 || data.getUint16(28, true) !== 32 ||
      data.getUint32(30, true) !== 0 || raw.length !== 54 + width * height * 4) return;
    return value;
  } catch { return; }
}
