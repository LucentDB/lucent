## 2025-02-23 - Exact Function Name Matching in SQL Guard Allows File Exfiltration Bypasses
**Vulnerability:** Exact string matching in `FORBIDDEN_FUNCTIONS` allowed DuckDB/Postgres function variants (`read_csv_auto`, `read_json_auto`, `read_ndjson`, `pg_read_file_all`, `pg_ls_tmpdir`) and file extensions (`.tsv`, `.ndjson`, `.jsonl`) to bypass the read-only guard and read arbitrary local files.
**Learning:** SQL engines continually add helper functions or alias variants that accept similar file paths or parameters.
**Prevention:** Use prefix/pattern matching (e.g., `starts_with("read_csv")`, `starts_with("pg_read_")`) in AST security guards alongside explicit exact lists to catch new or alternate function signatures.
