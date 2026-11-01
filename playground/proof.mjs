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
// where it can, and notes that would overlap move apart around their lines, as close to them as
// they fit, all between `floor` and `ceiling` (the floor wins when they do not fit).
export function stack(wanted, heights, gap, floor = -Infinity, ceiling = Infinity) {
  // Taking from each top the room the notes above it need turns "no overlap" into "tops in
  // order"; pooling each run that is out of order at its mean gives the closest such order.
  const room = [];
  let total = 0;
  for (const h of heights) {
    room.push(total);
    total += h + gap;
  }
  const runs = [];
  wanted.forEach((top, i) => {
    let run = { sum: top - room[i], n: 1 };
    while (runs.length && runs.at(-1).sum / runs.at(-1).n > run.sum / run.n) {
      const prev = runs.pop();
      run = { sum: prev.sum + run.sum, n: prev.n + run.n };
    }
    runs.push(run);
  });
  const high = ceiling - (total - gap);
  const tops = [];
  for (const run of runs) {
    const at = Math.max(floor, Math.min(high, run.sum / run.n));
    for (let k = 0; k < run.n; k++) tops.push(at + room[tops.length]);
  }
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
