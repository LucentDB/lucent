import { EditorState, type Extension } from '@codemirror/state';
import { CompletionContext } from '@codemirror/autocomplete';
import type { CompletionSource } from '@codemirror/autocomplete';

export const FIXTURE_TABLES = [
  {
    schema: 'bookings',
    name: 'airports_data',
    columns: [
      { name: 'airport_code', type_name: 'text' },
      { name: 'timezone', type_name: 'text' },
      { name: 'city', type_name: 'text' },
    ],
  },
  {
    schema: 'public',
    name: 'users',
    columns: [
      { name: 'id', type_name: 'int4' },
      { name: 'email', type_name: 'text' },
    ],
  },
  {
    schema: 'public',
    name: 'customers',
    columns: [
      { name: 'id', type_name: 'int4' },
      { name: 'name', type_name: 'text' },
    ],
  },
];

/** Returns every label the configured autocomplete sources would offer at pos. */
export function suggestionsAt(
  extensions: Extension[],
  doc: string,
  pos: number,
): string[] {
  const state = EditorState.create({
    doc,
    selection: { anchor: pos },
    extensions,
  });
  const ctx = new CompletionContext(state, pos, false);
  const out: string[] = [];
  for (const src of state.languageDataAt(
    'autocomplete',
    pos,
  ) as readonly CompletionSource[]) {
    const r = src(ctx);
    if (r && 'options' in r) out.push(...r.options.map((o) => o.label));
  }
  return out;
}
