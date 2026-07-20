// Minimal static file server, no dependencies. Serves the current directory
// so you can open the Comb batch-unstake app locally on your phone without
// needing to host it anywhere external.
//
// Run: node serve.js
// Then open http://127.0.0.1:8080 in Solflare's built-in browser.

const http = require("http");
const fs = require("fs");
const path = require("path");

const PORT = 8080;
const ROOT = __dirname;

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
};

http.createServer((req, res) => {
  let filePath = req.url === "/" ? "/index.html" : req.url;
  filePath = path.join(ROOT, decodeURIComponent(filePath.split("?")[0]));

  fs.readFile(filePath, (err, data) => {
    if (err) {
      res.writeHead(404);
      res.end("Not found");
      return;
    }
    const ext = path.extname(filePath);
    res.writeHead(200, { "Content-Type": MIME[ext] || "application/octet-stream" });
    res.end(data);
  });
}).listen(PORT, "127.0.0.1", () => {
  console.log(`Serving at http://127.0.0.1:${PORT}`);
  console.log("Open that URL inside Solflare's built-in browser tab.");
  console.log("Press Ctrl+C to stop the server when you're done testing.");
});
