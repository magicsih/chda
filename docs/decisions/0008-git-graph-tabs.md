# Read-only Git graph tabs

Status: accepted

The repository sidebar needs an in-app commit ancestry graph alongside terminals (#107). A graph must not create a terminal process or accept terminal split/input commands.

`TabContent` explicitly separates terminal split trees from repository graphs. Terminal focus and layout are available only for terminal tabs; both types share ordering, names, repository grouping, activation and closing. Session snapshots retain the previous terminal JSON fields and encode graph tabs with a `graph` repository path. Restoring a graph starts a new query.

`chda-git` reads local/remote branches, tags and HEAD using the existing gix dependency. An owning topological iterator keeps parents below their children, including merge commits and commits with inconsistent clocks. Reference names are captured before peeling so symbolic remote HEAD labels and annotated tags retain their identities. Non-commit tags are outside the ancestry graph.

`chda-core` exposes plain commit/page data and graph row geometry. Its query boundary retains the iterator between 200-commit pages. Initialization, ancestry traversal and subject decoding run on background tasks. gix may traverse ancestry metadata to establish topological order before the first page, especially without a commit-graph cache; commit subjects and rendered rows are still fetched incrementally. No Git write operation is involved.

The UI uses a virtual list and asks for the next page near the current page's end. Lane state persists across pages. Refresh replaces the query and increments a generation; late results from an older generation are discarded. Closing drops the graph entity, and weak background callbacks cannot reinsert a tab. Errors offer a fresh query through Retry.

Validation includes real Git histories with merges, annotated tags, remote symbolic HEAD, detached HEAD, unborn branches and 405 commits; lane joins and page continuity; mixed session round trips and legacy terminal JSON; and GPUI scenarios for menu opening, deduplication, terminal isolation, refresh invalidation, scrolling and retry.

References: [gix Repository](https://docs.rs/gix/0.88.0/gix/struct.Repository.html), [gix topological traversal](https://docs.rs/gix-traverse/0.62.0/gix_traverse/commit/topo/struct.Builder.html), pinned GPUI `uniform_list` and painting examples.
