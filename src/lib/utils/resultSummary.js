export function resultSummary(result) {
  if (result?.rows_affected != null)
    return `${result.rows_affected} rows affected`;
  return `${result?.row_count ?? 0} rows`;
}
