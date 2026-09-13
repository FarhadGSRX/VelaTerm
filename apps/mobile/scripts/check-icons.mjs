import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { inflateSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';

export const sourceURL = new URL('../assets/icon-ios.svg', import.meta.url);
export const iconURL = new URL('../ios/App/App/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png', import.meta.url);
export const manifestURL = new URL('../assets/icon-ios.generated.json', import.meta.url);
export const sha256 = value => createHash('sha256').update(value).digest('hex');

// 验证最终 PNG 的尺寸、透明度及四边像素，不能只检查 SVG 的背景声明。
export function validateIcon(png) {
  if (!png.subarray(0, 8).equals(Buffer.from([137,80,78,71,13,10,26,10]))) throw new Error('图标不是 PNG');
  let header;
  const chunks = [];
  for (let offset = 8; offset + 12 <= png.length;) {
    const size = png.readUInt32BE(offset);
    const type = png.toString('ascii', offset + 4, offset + 8);
    const data = png.subarray(offset + 8, offset + 8 + size);
    if (data.length !== size) throw new Error('PNG 数据不完整');
    if (type === 'IHDR') header = data;
    if (type === 'IDAT') chunks.push(data);
    offset += size + 12;
  }
  if (!header || header.readUInt32BE(0) !== 1024 || header.readUInt32BE(4) !== 1024 || header[8] !== 8 || ![2,6].includes(header[9]) || header[12] !== 0) throw new Error('iOS 图标必须为 1024×1024、8 位非交错 RGB/RGBA PNG');
  const channels = header[9] === 2 ? 3 : 4;
  const stride = 1024 * channels;
  const raw = inflateSync(Buffer.concat(chunks));
  if (raw.length !== (stride + 1) * 1024) throw new Error('PNG 像素数据长度不正确');
  let previous = Buffer.alloc(stride);
  for (let y = 0; y < 1024; y++) {
    const filter = raw[y * (stride + 1)];
    if (filter > 4) throw new Error('PNG 滤波格式不正确');
    const row = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? row[i - channels] : 0;
      const b = previous[i];
      const c = i >= channels ? previous[i - channels] : 0;
      const p = a + b - c;
      const distances = [Math.abs(p-a), Math.abs(p-b), Math.abs(p-c)];
      const paeth = distances[0] <= distances[1] && distances[0] <= distances[2] ? a : distances[1] <= distances[2] ? b : c;
      row[i] = (row[i] + [0, a, b, Math.floor((a+b)/2), paeth][filter]) & 255;
    }
    for (let x = 0; x < 1024; x++) {
      const i = x * channels;
      if (channels === 4 && row[i+3] !== 255) throw new Error('iOS 图标不能包含透明像素或预裁切圆角');
      if ((x === 0 || y === 0 || x === 1023 || y === 1023) && Math.max(row[i], row[i+1], row[i+2]) > 80) throw new Error('当前深色品牌图标的背景必须铺满四边，禁止白边或留白');
    }
    previous = row;
  }
}

export async function checkIcons() {
  const [source, icon, manifest] = await Promise.all([readFile(sourceURL), readFile(iconURL), readFile(manifestURL, 'utf8').then(JSON.parse)]);
  validateIcon(icon);
  if (manifest.sourceSHA256 !== sha256(source) || manifest.iconSHA256 !== sha256(icon)) throw new Error('图标母版与产物不一致，请运行 pnpm icons 重新生成');
  console.log('iOS 图标检查通过：满版、不透明，母版与产物一致。');
}
if (process.argv[1] === fileURLToPath(import.meta.url)) await checkIcons();
