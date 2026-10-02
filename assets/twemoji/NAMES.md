# Emoji picker names

`names.tsv` contains 3,953 Unicode 17.0 fully qualified emoji and components
that have artwork in the bundled Twemoji atlas. Columns are the original
fully qualified Unicode sequence and its lowercase English name, in Unicode's
recommended CLDR keyboard order. This preserves presentation selectors when
inserting into a draft. The 56 remaining atlas entries are not listed as RGI
emoji/components in the Unicode 17.0 test data; the message renderer still
supports them.

Source: <https://www.unicode.org/Public/17.0.0/emoji/emoji-test.txt>, downloaded
September 10, 2026. Source SHA-256:
`1d8a944f88d7952f7ef7c5167fef3c67995bcae24543949710231b03a201acda`.
Copyright © 2025 Unicode, Inc. Used under the [Unicode License v3](LICENSE-UNICODE).

To reproduce: parse each non-comment source line's codepoints, status and
English name; retain `fully-qualified` and `component` statuses whose sequence
matches `index.tsv` after removing U+FE0F; write the original sequence, a tab,
and the lowercased name. Keep source order. No runtime network lookup or new
runtime dependency is needed.

`discord-shortcodes.tsv` keeps that same order and adds Discord's primary name
and comma-separated colon aliases. It was generated from the public Discord
Canary client map captured September 23, 2026
(`vnd-emoji.ca4ac71ece8afd45`, map timestamp
`2026-09-23T09:04:12.310152+00:00`) through
<https://static.emzi0767.com/misc/discordEmojiMap.min.json>. The downloaded map
SHA-256 was
`be71a1013f29ea33ede02b3bf4e1ea649bfa28336d5e791dea78dd6577d42421`.
It covers 3,781 bundled sequences; the 172 Unicode 17 sequences absent from
that Discord client map retain normalized CLDR names until Discord names them.
