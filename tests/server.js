const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');

const html = `<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/static/style.css"></head><body>
<main class="wrap"><header class="topbar"><div class="brand">Todo</div><div class="person">Test</div></header>
<section class="panel"><div class="heading"><div><h1>Mes tâches</h1><div id="count" class="count"></div></div><button id="new-task" class="new">+ Nouvelle tâche</button></div><div id="tree" class="tree"></div></section></main>
<div id="modal" class="overlay hidden"><form id="task-form" class="modal"><h2 id="modal-title"></h2><input type="hidden" name="id"><input type="hidden" name="parent_id"><input id="task-title" name="title" required><textarea name="description"></textarea><div class="modal-actions"><button id="cancel" type="button">Annuler</button><button type="submit">Enregistrer</button></div></form></div>
<script src="/static/app.js" defer></script></body></html>`;

http.createServer((req, res) => {
  if (req.url === '/') { res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' }); res.end(html); return; }
  const files = { '/static/app.js': ['app.js', 'text/javascript'], '/static/style.css': ['style.css', 'text/css'] };
  const entry = files[req.url];
  if (!entry) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'Content-Type': `${entry[1]}; charset=utf-8` });
  fs.createReadStream(path.join(__dirname, '..', 'static', entry[0])).pipe(res);
}).listen(4187, '127.0.0.1');
