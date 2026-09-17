# Issue tracker: GitHub

Issues and specs live in this repository's GitHub Issues. Use the `gh` CLI for
tracker operations. Infer the repository from `git remote -v`; `gh` does this
automatically when run inside the checkout.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body-file <path>`.
- **Read an issue**: `gh issue view <number> --json number,title,body,labels,comments,state,url`.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments` with appropriate `--label` and `--state` filters. Set `--limit` to cover the queue being processed; the default is 30.
- **Comment**: `gh issue comment <number> --body-file <path>`.
- **Apply or remove labels**: `gh issue edit <number> --add-label "..."` or `--remove-label "..."`.
- **Close**: `gh issue close <number>`.

For multiline bodies, write the exact Markdown to a file and pass it with
`--body-file`. Preserve real newlines and literal code examples.

## Pull requests as a triage surface

**PRs as a request surface: no.** Set to `yes` if external PRs should be treated
as feature requests; `/triage` reads this flag.

When enabled, use the same labels and states as issues:

- **Read**: `gh pr view <number> --comments` and `gh pr diff <number>`.
- **List**: `gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments`. Keep external authors with association `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR`, or `NONE`.
- **Comment, label, or close**: `gh pr comment <number> --body-file <path>`, `gh pr edit <number> --add-label "..."` or `--remove-label "..."`, and `gh pr close <number>`.

GitHub shares one number space across issues and PRs. For an ambiguous `#42`,
resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## Skill operations

- **Publish to the issue tracker**: create a GitHub issue.
- **Fetch the relevant ticket**: read the issue body, labels, and comments.

## Wayfinding operations

Used by `/wayfinder`. The map is one issue with child issues as tickets.

- **Map**: an issue labelled `wayfinder:map`, containing Notes, Decisions-so-far, and Fog.
- **Child ticket**: link it to the map as a GitHub sub-issue using `gh api`. If sub-issues are unavailable, add a task-list entry in the map and `Part of #<map>` in the child's body. Use `wayfinder:<type>` labels (`research`, `prototype`, `grilling`, or `task`). Create these labels when first needed.
- **Blocking**: use native issue dependencies. Add an edge with `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`. Obtain the blocker's numeric database ID with `gh api repos/<owner>/<repo>/issues/<number> --jq .id`. If dependencies are unavailable, put `Blocked by: #<number>` at the top of the child body.
- **Frontier**: inspect the map's open children in map order. Skip assigned tickets and tickets with open blockers. Native `issue_dependencies_summary.blocked_by` counts open blockers; for the fallback, check the referenced issues' states.
- **Claim**: `gh issue edit <number> --add-assignee @me`.
- **Resolve**: record the result in a comment, close the child, and add a concise result plus a link to the map's Decisions-so-far.
