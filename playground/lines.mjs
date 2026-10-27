// A textarea breaks a row at LF only, whatever the dialect: its value has no CR, and a browser does
// not break at U+2028 or U+2029, which JSON5 counts as line ends. The gutter and the highlights
// follow the rows, and the wasm gives each problem's UTF-16 index to find its row.

// The index where each row of `text` starts.
export function rowStarts(text) {
  const starts = [0];
  for (let i = text.indexOf("\n"); i >= 0; i = text.indexOf("\n", i + 1)) starts.push(i + 1);
  return starts;
}

// The 1-based row that holds `index`.
export function rowOf(starts, index) {
  let lo = 0;
  let hi = starts.length;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (starts[mid] <= index) lo = mid;
    else hi = mid;
  }
  return lo + 1;
}
