// A share link keeps the dialect and the text in the URL fragment, as `#dialect:data`, where data
// is the text as UTF-8, deflated and in base64url. Browsers never send the fragment to a server.

export const DIALECTS = ["json", "i-json", "jsonc", "jwcc", "json5", "jsonl"];
// A link that inflates past this many bytes is refused, so a crafted one cannot hang the tab.
const MAX_BYTES = 8 << 20;

async function pipe(bytes, stream) {
  const reader = new Blob([bytes]).stream().pipeThrough(stream).getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > MAX_BYTES) {
      await reader.cancel();
      throw new RangeError("too large");
    }
    chunks.push(value);
  }
  return new Uint8Array(await new Blob(chunks).arrayBuffer());
}

// The fragment for `text` in `dialect`, without the `#`.
export async function encode(dialect, text) {
  const bytes = await pipe(new TextEncoder().encode(text), new CompressionStream("deflate-raw"));
  let binary = "";
  // In slices: String.fromCharCode takes its bytes as arguments, and those are limited.
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  const data = btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  return `${dialect}:${data}`;
}

// The dialect and text in `fragment` (with or without its `#`), or null when it holds no share
// link this page made.
export async function decode(fragment) {
  const match = /^#?([a-z0-9-]+):([A-Za-z0-9_-]+)$/.exec(fragment);
  if (!match || !DIALECTS.includes(match[1])) return null;
  try {
    const binary = atob(match[2].replaceAll("-", "+").replaceAll("_", "/"));
    const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
    const text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
      await pipe(bytes, new DecompressionStream("deflate-raw")),
    );
    return { dialect: match[1], text };
  } catch {
    return null;
  }
}
