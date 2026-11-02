slug: tiffgraph
shelf: photography
default_edition: full

# Publication contract

This repository owns the code, the manual in `docs/tiffgraph-manual.md`, the cover
and headboards in `cover/`, and the blog post in `docs/blog/tiffgraph/`. FirstPair
owns catalog entries, stable download and reader routes, and Blob manifests.
Follow the shared `~/src/firstpair/AGENTS.md` publication rules.

The manual is by Alexy Khrabrov with Claude as co-author. The code is MIT.

`cover/tiffgraph-cover.png` is the book cover and `cover/tiffgraph-headboard.png`,
without text, is the shelf and blog headboard; both are the user's art.
`cover/cover.typ` draws the `*-drawn.png` images: a region-role diagram used as a
figure in the manual and the post, and an alternative cover and headboards.

```sh
PATH=/usr/local/bin:$PATH ~/src/firstpair/publishing/scripts/build-library-book.sh --repo-root ~/src/tiffgraph
cd ~/src/firstpair
npm run library:publish -- ~/src/tiffgraph --source https://github.com/firstpair/tiffgraph
```
