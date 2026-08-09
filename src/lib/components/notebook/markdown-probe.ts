/**
 * Signals that a string is *intended* as markdown. Used by AI cells, whose prompts
 * are usually prose: rendering everything through marked wraps a plain question in
 * <p> and mangles any stray # or *.
 *
 * Deliberately conservative — a false negative shows plain text, which is fine; a
 * false positive garbles the user's words.
 */
const MARKDOWN_SIGNALS: RegExp[] = [
  /^\s{0,3}#{1,6}\s/m, // ATX heading (space required)
  /^\s*```/m, // fenced code
  /^\s*~~~/m, // fenced code, tilde form
  /^\s{0,3}[-*+]\s+\S/m, // bullet list
  /^\s{0,3}\d+[.)]\s+\S/m, // ordered list
  /^\s{0,3}>\s/m, // blockquote
  /^\s*\|.*\|\s*$/m, // table row
  /^\s{0,3}(?:[-*_]\s*){3,}$/m, // thematic break
  /\*\*[^*\n]+\*\*/, // bold
  /~~[^~\n]+~~/, // strikethrough
  /`[^`\n]+`/, // inline code
  /\[[^\]\n]+\]\([^)\s]+\)/, // link
];

export function looksLikeMarkdown(text: string): boolean {
  if (!text || !text.trim()) return false;
  return MARKDOWN_SIGNALS.some((re) => re.test(text));
}
