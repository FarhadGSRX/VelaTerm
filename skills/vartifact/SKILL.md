---
name: vartifact
description: >-
  Show the user a visual result as a live page in VelaTerm. Use when the user asks to see, mock up, sketch or
  prototype a screen, page, form, chart or diagram, rather than to read code: write one self-contained HTML file
  and open it with `vopen`, which renders it beside this conversation and reloads it on every save. Available only
  inside VelaTerm-hosted local sessions.
argument-hint: "[what to show]"
allowed-tools: Bash(vopen:*)
---

# vartifact

VelaTerm previews HTML pages as artifacts. A page opened with `vopen` from this conversation appears in a panel
beside it, reloads whenever the file changes, and keeps each saved version so the user can step back through them.
Errors the page logs show in the panel's console, and the user can send them back to you.

User input:

$ARGUMENTS

## Step 1: Decide where the page goes

- Use the path the user named. Otherwise put it next to the code it is about, or in the working directory, with a
  short descriptive name such as `clone-dialog.html`.
- Keep editing the same file across rounds. A new file starts a new history; the same file builds one.

## Step 2: Write the page

- One self-contained HTML file: inline `<style>` and `<script>`. Files beside it (`./style.css`, `./app.js`,
  images) also load, if the page really needs them.
- The preview starts **offline**: requests to the internet, including CDNs and web fonts, are blocked unless the
  user allows the network for that file. Use system fonts and inline everything the page needs.
- Use realistic sample content, not lorem ipsum, and make the layout work at narrow widths: the panel is often
  half a screen.

## Step 3: Open it

```bash
vopen <absolute path>
```

Run it once; later edits reload the preview on their own, so there is no need to run it again.

## Step 4: Report back

Say in one or two lines what the page shows and what to look at. Do not paste the HTML into the conversation; the
user is looking at the page.

## Notes

- `vopen` needs a VelaTerm-hosted local session. If it reports that it is not inside one, tell the user the
  preview is only available in VelaTerm and give the file path instead.
- If the user reports errors from the preview, fix the page in place.
