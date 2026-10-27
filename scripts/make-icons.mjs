// Writes the placeholder app icons (app/src-tauri/icons/): a red disc, as PNG at 32, 128 and
// 256 pixels and as a classic Windows ICO holding 16, 32 and 48 pixel images. Tauri needs an
// .ico on Windows and a .png elsewhere at build time; the tray reuses the embedded icon. Uses only
// Node's standard library, so a fresh clone can regenerate the files:
//
//   node scripts/make-icons.mjs [output directory]
//
// The packaging module replaces these with the designed icons.

import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { deflateSync } from 'node:zlib'

const outDir = process.argv[2] ?? join('app', 'src-tauri', 'icons')

const RED = [214, 40, 40]
const DARK = [160, 24, 24]

/** RGBA pixels of a red disc with a darker rim, anti-aliased at the edge. */
function disc(size) {
  const rgba = new Uint8Array(size * size * 4)
  const centre = size / 2
  const radius = size / 2 - 0.5
  const rim = Math.max(1, size / 12)
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dx = x + 0.5 - centre
      const dy = y + 0.5 - centre
      const d = Math.sqrt(dx * dx + dy * dy)
      // Coverage of the pixel by the disc: 1 inside, 0 outside, a linear ramp across one pixel.
      const coverage = Math.min(1, Math.max(0, radius - d + 0.5))
      if (coverage <= 0) continue
      const colour = d > radius - rim ? DARK : RED
      const i = (y * size + x) * 4
      rgba[i] = colour[0]
      rgba[i + 1] = colour[1]
      rgba[i + 2] = colour[2]
      rgba[i + 3] = Math.round(coverage * 255)
    }
  }
  return rgba
}

const CRC_TABLE = new Uint32Array(256).map((_, n) => {
  let c = n
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
  return c >>> 0
})

function crc32(bytes) {
  let c = 0xffffffff
  for (const b of bytes) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

function u32be(n) {
  const b = Buffer.alloc(4)
  b.writeUInt32BE(n >>> 0)
  return b
}

function chunk(type, data) {
  const typeBytes = Buffer.from(type, 'ascii')
  const crc = crc32(Buffer.concat([typeBytes, data]))
  return Buffer.concat([u32be(data.length), typeBytes, data, u32be(crc)])
}

/** A PNG (8-bit RGBA, no interlace) from RGBA pixels. */
function png(size, rgba) {
  const raw = Buffer.alloc((size * 4 + 1) * size)
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0 // filter type None
    Buffer.from(rgba.buffer, y * size * 4, size * 4).copy(raw, y * (size * 4 + 1) + 1)
  }
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(size, 0)
  ihdr.writeUInt32BE(size, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // colour type RGBA
  ihdr[10] = 0
  ihdr[11] = 0
  ihdr[12] = 0
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

/** A classic ICO: 32-bit BGRA images with an AND mask, bottom-up rows. */
function ico(sizes) {
  const images = sizes.map((size) => {
    const rgba = disc(size)
    const maskRow = Math.ceil(size / 32) * 4
    const header = Buffer.alloc(40)
    header.writeUInt32LE(40, 0)
    header.writeInt32LE(size, 4)
    header.writeInt32LE(size * 2, 8) // XOR and AND masks stacked
    header.writeUInt16LE(1, 12)
    header.writeUInt16LE(32, 14)
    header.writeUInt32LE(0, 16)
    header.writeUInt32LE(size * size * 4 + maskRow * size, 20)
    const xor = Buffer.alloc(size * size * 4)
    const and = Buffer.alloc(maskRow * size)
    for (let y = 0; y < size; y++) {
      const srcRow = size - 1 - y // bottom-up
      for (let x = 0; x < size; x++) {
        const s = (srcRow * size + x) * 4
        const d = (y * size + x) * 4
        xor[d] = rgba[s + 2]
        xor[d + 1] = rgba[s + 1]
        xor[d + 2] = rgba[s]
        xor[d + 3] = rgba[s + 3]
        if (rgba[s + 3] === 0) and[y * maskRow + (x >> 3)] |= 0x80 >> (x & 7)
      }
    }
    return { size, data: Buffer.concat([header, xor, and]) }
  })
  const dir = Buffer.alloc(6)
  dir.writeUInt16LE(0, 0)
  dir.writeUInt16LE(1, 2)
  dir.writeUInt16LE(images.length, 4)
  let offset = 6 + 16 * images.length
  const entries = images.map(({ size, data }) => {
    const e = Buffer.alloc(16)
    e[0] = size === 256 ? 0 : size
    e[1] = size === 256 ? 0 : size
    e[2] = 0
    e[3] = 0
    e.writeUInt16LE(1, 4)
    e.writeUInt16LE(32, 6)
    e.writeUInt32LE(data.length, 8)
    e.writeUInt32LE(offset, 12)
    offset += data.length
    return e
  })
  return Buffer.concat([dir, ...entries, ...images.map((i) => i.data)])
}

mkdirSync(outDir, { recursive: true })
writeFileSync(join(outDir, '32x32.png'), png(32, disc(32)))
writeFileSync(join(outDir, '128x128.png'), png(128, disc(128)))
writeFileSync(join(outDir, 'icon.png'), png(256, disc(256)))
writeFileSync(join(outDir, 'icon.ico'), ico([16, 32, 48]))
console.log(`icons written to ${outDir}`)
