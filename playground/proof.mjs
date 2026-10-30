// Pure helpers for setting the text as a proof: where a mark ends, where the margin notes go, and
// how a measured time reads.

const WORD = /[\w$.+-]/;

// Where the token that starts at `offset` in `row` ends: a quoted string to its closing quote (or
// the end of the row), a run of word characters, or else one code point.
export function tokenEnd(row, offset) {
  if (offset >= row.length) return row.length;
  const c = row[offset];
  if (c === '"' || c === "'") {
    for (let i = offset + 1; i < row.length; i++) {
      if (row[i] === "\\") i++;
      else if (row[i] === c) return i + 1;
    }
    return row.length;
  }
  if (WORD.test(c)) {
    let i = offset + 1;
    while (i < row.length && WORD.test(row[i])) i++;
    return i;
  }
  return offset + String.fromCodePoint(row.codePointAt(offset)).length;
}

// The top of each note, given the top it wants (sorted) and its height: a note keeps its line
// unless the one above reaches it, then it goes `gap` below that one.
export function stack(wanted, heights, gap) {
  const tops = [];
  let floor = -Infinity;
  wanted.forEach((top, i) => {
    tops.push(Math.max(top, floor));
    floor = tops[i] + heights[i] + gap;
  });
  return tops;
}

// A time measured with performance.now(), whose clock moves in steps of `resolution` ms: browsers
// coarsen it, so a time under one step is given as a bound, not a number.
export function formatMs(ms, resolution) {
  if (ms < resolution) return `under ${+resolution.toPrecision(1)} ms`;
  const value = ms < 1 ? +ms.toPrecision(2) : ms < 10 ? +ms.toFixed(1) : Math.round(ms);
  return `${value.toLocaleString("en")} ms`;
}

// The number of lines in a text of `length` characters whose rows start at `starts`: a final line
// break ends the last line rather than starting one.
export function lineCount(starts, length) {
  return starts.length > 1 && starts.at(-1) === length ? starts.length - 1 : starts.length;
}
