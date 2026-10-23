// Line ends as jsonvet counts them: JSON5 also ends a line at U+2028 and U+2029.
export const lineEnd = (dialect) =>
  dialect === "json5" ? /\r\n|[\r\n\u2028\u2029]/g : /\r\n|[\r\n]/g;

// The UTF-16 index of a 1-based line and a column in code points.
export function indexOf(text, line, column, dialect) {
  const ends = lineEnd(dialect);
  let start = 0;
  for (let l = 1; l < line && ends.exec(text); l++) start = ends.lastIndex;
  let i = start;
  for (let c = 1; c < column && i < text.length; c++) i += text.codePointAt(i) > 0xffff ? 2 : 1;
  return i;
}
