# Lucent demo video — recording script

~90 seconds. Record at 1440p or 1080p, app maximized, clean desktop (notifications off,
no PII). Read the "expected on-screen" lines as captions/voiceover cues — they state the
exact text the viewer should see, so the seed data below is **not optional**: the beats
assume those stored values.

## Setup (before recording)

1. Start the local dev container: `docker run -d --name lucent-dev-pg -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres` and create the demo database with a small churn schema:

   ```sql
   CREATE DATABASE lucent_demo;
   \c lucent_demo

   CREATE TABLE customers (
     id          serial PRIMARY KEY,
     name        text NOT NULL,
     email       text NOT NULL UNIQUE,
     status      text NOT NULL,          -- values 'Active' / 'Churned' — STORED CASING matters
     churned_at  timestamptz
   );
   CREATE TABLE orders (
     id          serial PRIMARY KEY,
     customer_id int NOT NULL REFERENCES customers(id),
     amount      numeric(10,2) NOT NULL,
     placed_at   timestamptz NOT NULL DEFAULT now()
   );

   INSERT INTO customers (name, email, status, churned_at) VALUES
     ('Ada Lovelace', 'ada@example.com', 'Active',  NULL),
     ('Grace Hopper', 'grace@example.com', 'Active', NULL),
     ('Margaret Hamilton', 'margaret@example.com', 'Churned', now() - interval '14 days'),
     ('Katherine Johnson', 'katherine@example.com', 'Active', NULL);

   INSERT INTO orders (customer_id, amount) VALUES
     (1, 129.00), (2, 89.50), (3, 240.00), (4, 19.99);

   -- Gives the schema browse a View group (empty groups don't render).
   CREATE VIEW active_customers AS
     SELECT id, name, email FROM customers WHERE status = 'Active';
   ```

2. In the app: **AI Settings** → configure a provider (OpenAI / Anthropic / local Ollama).
3. Do a dry run of Beat 3 and Beat 4 offline first — LLM output wording varies; the *cards,
   tool calls, and numbers* are deterministic and are what the script asserts.

## The script

### Beat 1 — Connect to Postgres (0:00–0:10, ~10s)

| Step | Expected on-screen |
|---|---|
| Launch Lucent. | Landing page: "Quick Connect" form (host / port / database / user / password) + connection list on the left. |
| Fill: host `localhost`, port `5432`, database `lucent_demo`, user `postgres`, password `postgres`. | Form fields populate as typed. |
| Click **Connect**. | `lucent_demo` appears in the connection list with a connected state; schema tree loads on the left. |

### Beat 2 — Browse the schema (0:10–0:25, ~15s)

| Step | Expected on-screen |
|---|---|
| Expand the schema tree: `lucent_demo` → `public`. | Group headers appear: **Tables** (2), **Views** (1), **Sequences** (2) — each with a count badge. No **Functions** group (none in the seed; empty groups don't render). |
| Expand the **Tables** group. | `customers`, `orders` as the leaf rows. |
| Click `customers`. | A query tab opens with a preview grid — the column headers **id, name, email, status, churned_at** and the four seed rows; the `status` column shows `Active` / `Churned`. |
| Click `orders`. | Grid headers: `id`, `customer_id`, `amount`, `placed_at` — `customer_id` holds the ids (1–4) that link orders back to `customers`; that's the schema the AI copilot will reason over. |

### Beat 3 — Ask the AI, wrong literal → grounded answer (0:25–0:55, ~30s)

This is the money shot. The stored value is `Churned` (capitalized); the model will write
`'churned'`. The preflight probe corrects it before you ever see a wrong answer — you see
the *correction happen live* in the tool calls.

| Step | Expected on-screen |
|---|---|
| `Cmd/Ctrl+Shift+A` to open the AI chat. | Chat panel opens with the input "Ask anything about lucent_demo…". |
| Type and send: `which customers churned`. | Thinking block starts; then tool-call cards stream in (schema retrieval, then the query). |
| Watch the first query's card. | **Input** — the SQL, including `status = 'churned'` (wrong casing). **Output** — the same SQL as the header with an **empty result table: the column headers only, zero rows**. |
| Watch the agent self-correct. | A second query card: `status = 'Churned'` — this one's result table has **one row: `Margaret Hamilton`** (plus her `churned_at` date). |
| Final answer streams in. | Grounded answer, e.g.: *"1 customer has churned: Margaret Hamilton (churned 14 days ago, 1 order, $240.00)."* |

The empty result is the tell — hold on it for a beat before the corrected query lands;
it's the visible proof that the agent checks its literals against the real data. If your
model nails the casing on the first try (newer models sometimes do), seed one customer
as `CHURNED` in ALL CAPS instead so the miss is guaranteed.

### Beat 4 — DML approval: blast radius + confirmation (0:55–1:20, ~25s)

Headline feature of this release: writes are staged, reviewed, executed — and the agent
confirms the outcome in the same thread (C1).

| Step | Expected on-screen |
|---|---|
| In the same chat, send: `mark customer 1 as churned`. | Agent streams tool calls: `preview_dml`, then the conversation **pauses**. |
| Watch the `preview_dml` tool card. | Description **"UPDATE on customers — ~1 rows"** with the staged SQL underneath — the agent's own estimate, from a read-only `SELECT count(*)` it ran first. |
| An approval card appears in the thread: | 🔒 **"Review DML Statement"** — description, the staged SQL, and the blast-radius line: **⚠️ Estimated rows affected: 1**. |
| Click **Execute**. | The card flips green: ✓ **"DML Executed"** — **1 rows affected**. The chat un-pauses. |
| Watch the thread. | The agent's confirmation streams into the same message, e.g.: *"Done — customer 1 (Ada Lovelace) is now marked as churned."* |

Resist cutting from the card to something else — the pause → review → execute → confirm
loop is the whole story. (Worth knowing: cancel is one click if you flub the recording —
the card has a **Cancel** button that aborts with no SQL executed.)

### Beat 5 — Notebook cells (1:20–1:30, ~10s)

| Step | Expected on-screen |
|---|---|
| Press `Cmd/Ctrl+Shift+N` (or **File → New Notebook**) to create a notebook tab. | A new notebook tab (📓) opens with an empty cell gutter. |
| Add a markdown cell, type `# Churn analysis`, press `Cmd/Ctrl+Enter` to render it. | Rendered heading — markdown and SQL coexist in one document. |
| Add a SQL cell: `SELECT status, count(*) FROM customers GROUP BY status;` and run it. | Inline results grid below the cell — `Active 2` / `Churned 2` (customer 1 churned in Beat 4). |

### Wrap (1:30)

Freeze on the notebook for the last two seconds, then fade out.

---

## After recording

Convert to a GIF (`ffmpeg -i lucent-demo.mov -vf "fps=24,scale=960:-1" lucent-demo.gif`) or
link the video, then replace the placeholder in the README demo slot — the `## Demo`
section, lines 19–25 of `README.md` (the `🎬 Demo coming soon` block):

```markdown
<div align="center">
  <img src="docs/demo.gif" alt="Lucent demo — connect, ask, approve, analyze" />
</div>
```

Commit the asset with `docs: demo video` (and bump nothing else — no code changes).
