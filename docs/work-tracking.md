# Tracking contract

Use todo.txt notation in todo.md: optional priority and creation date, task text,
+project, @context, id:tNNN, order:NNN, depends: IDs and a link to the detailed note.
Task status lives there, not in duplicate note checklists. Allocate IDs once and
never recycle them. Completed lines start with x and completion date; move them
to done-YYYY-MM.md. The repository-local .todo/config supports upstream todo.txt.

Notes use zk, stable eight-character IDs and Markdown links. Plans describe exits;
investigations retain hypotheses and contrary evidence; decisions record tradeoffs;
milestones name exact tested source and qualifications. Raw captures belong in
.artifacts, not copied into notes. Run zk index and inspect broken links.
