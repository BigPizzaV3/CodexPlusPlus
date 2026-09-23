# taskctl CLI

`taskctl` emits JSON. Add `--json` when making the output contract explicit.

## Context and projects

```bash
taskctl context current [--cwd PATH] [--json]
taskctl project list [--json]
taskctl project create --name NAME [--id ID] [--workspace-path PATH] [--json]
taskctl project map PROJECT_ID --workspace-path PATH [--json]
```

Use `--workspace-path` to associate a project with a local repository. `context current` chooses the most specific project whose workspace contains the current directory, then falls back to the `local` project.

Set `CODEX_TASKBOARD_URL` to override the default local API origin, `http://127.0.0.1:47823`.

For a shared cloud board, keep `taskctl` pointed at the loopback companion and configure the upstream HTTPS origin through it:

```bash
taskctl cloud login --url HTTPS_ORIGIN --actor-name NAME [--json]
taskctl cloud status [--json]
taskctl project list [--json]
taskctl project map PROJECT_ID --workspace-path /absolute/local/path [--json]
taskctl cloud logout [--json]
```

`cloud login` reads the shared password from a private `Shared key:` prompt. The actor name is the display attribution sent through Basic Authentication. The companion stores its configuration with mode `0600`; project mappings stay on the current device and can differ between collaborators. In cloud mode, failed upstream writes fail rather than falling back to or double-writing the local SQLite database.

Every issue or comment write must be attributed to a Codex conversation. In Codex, `taskctl` reads the current conversation from `CODEX_THREAD_ID`. Outside Codex, pass `--thread-id ID` explicitly. An explicit option takes precedence over the environment. Read commands do not require a conversation id.

Every successful command writes one JSON object with `schemaVersion` to stdout. The current schema version is `2`. Errors write one JSON object to stderr. Exit codes are `0` for success, `2` for invalid input, `3` when the service is unavailable, `4` for API or response errors, and `5` for conflicts.

## Read issues

```bash
taskctl issue list [--project PROJECT_ID] [--status STATUS] [--json]
taskctl issue get ID [--json]
```

## Create issues

```bash
taskctl issue create \
  --project PROJECT_ID \
  --title TITLE \
  [--description TEXT | --description-file FILE] \
  [--status STATUS] \
  [--priority PRIORITY] \
  [--labels a,b] \
  [--thread-id ID] \
  [--git-branch BRANCH] \
  [--worktree-path PATH] \
  [--worktree-branch BRANCH] \
  [--due-date YYYY-MM-DD] \
  [--recurrence-interval N --recurrence-unit day|week|month|year] \
  [--json]
```

Statuses are `backlog`, `todo`, `in_progress`, `in_review`, `blocked`, `done`, and `canceled`. Priorities are `none`, `urgent`, `high`, `medium`, and `low`.

Issues created through `taskctl` are assigned to Codex Agent by default. Other CLI writes preserve the existing assignee.

## Update issues

Read the issue immediately before a write and pass its `version` with `--if-version`.

```bash
taskctl issue update ID \
  [--title TITLE] \
  [--description TEXT | --description-file FILE] \
  [--status STATUS] \
  [--priority PRIORITY] \
  [--labels a,b] \
  [--thread-id ID] \
  [--git-branch BRANCH] \
  [--worktree-path PATH] \
  [--worktree-branch BRANCH] \
  [--due-date YYYY-MM-DD] \
  [--recurrence-interval N --recurrence-unit day|week|month|year] \
  [--if-version N] \
  [--json]

taskctl issue move ID --status STATUS [--thread-id ID] [--if-version N] [--json]
taskctl issue archive ID [--thread-id ID] [--if-version N] [--json]
taskctl issue restore ID [--thread-id ID] [--if-version N] [--json]
```

Use `issue move` to set `in_progress` before implementation and `in_review` after implementation and self-verification. Codex must not move work directly from `in_progress` to `done`; use `done` only after the user explicitly confirms acceptance or explicitly asks to mark the issue complete. Use `blocked` when work cannot continue and `canceled` when it will not continue. On a version conflict, fetch the issue again and reconcile before retrying.

Use either `--git-branch` or `--worktree-path`/`--worktree-branch`; an issue has only one development context. Issue JSON stores it as `developmentContext`, either `{ "type": "branch", "branch": "..." }` or `{ "type": "worktree", "path": "...", "branch": "..." }`. Its singular `threadId` is the Codex conversation that most recently created or changed the issue itself. Recurrence requires a due date.

## Issue relations

Read the anchor issue immediately before adding or removing a relation and use its current version. Relation writes require Codex conversation attribution like every other issue write.

```bash
taskctl issue relation add ISSUE_ID \
  --type parent \
  --issue PARENT_ISSUE_ID \
  [--thread-id ID] \
  [--if-version N] \
  [--json]

taskctl issue relation add ISSUE_ID \
  --type blocks|blocked_by|related \
  --issue RELATED_ISSUE_ID \
  [--thread-id ID] \
  [--if-version N] \
  [--json]

taskctl issue relation remove ISSUE_ID \
  --type parent|blocks|blocked_by|related \
  --issue RELATED_ISSUE_ID \
  [--thread-id ID] \
  [--if-version N] \
  [--json]
```

For `--type parent`, `ISSUE_ID` is the child and `PARENT_ISSUE_ID` is its parent. Adding another parent replaces the child's current parent atomically. To add an existing issue as a sub-issue of `LOCAL-6`, anchor the command on the child and pass `--issue LOCAL-6`.

For `blocks`, the anchor issue blocks the related issue. For `blocked_by`, the related issue blocks the anchor. `related` is symmetric. Self-relations, duplicates, parent cycles, and relations between different projects are rejected.

## Issue comments

Use the issue id to read or append comments. Comment updates and deletes require the latest comment `version` returned by `comment list`.

```bash
taskctl comment list ISSUE_ID [--json]
taskctl comment add ISSUE_ID --body TEXT [--thread-id ID] [--json]
taskctl comment update COMMENT_ID --body TEXT --if-version N [--thread-id ID] [--json]
taskctl comment delete COMMENT_ID --if-version N [--thread-id ID] [--json]
```

Each comment JSON object independently records the most recent conversation that created or changed that comment as `threadId`. Comment operations never change the parent issue's `threadId`.

## Download inline images

Issue descriptions and comments may contain inline images at exact positions in their Markdown:

```markdown
![alt text](/api/attachments/ATTACHMENT_ID/content)
```

Download an inline image to an explicit local path before inspecting it:

```bash
taskctl attachment download ATTACHMENT_ID --output PATH [--json]
```

The command writes the response body as binary data and returns the absolute output path, content type, and size in its JSON result. Choose the output filename yourself; `taskctl` does not infer or append an extension.
