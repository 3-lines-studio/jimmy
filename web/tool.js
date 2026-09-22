
const CD = /^cd\s+(\S+)(?:\s+2>\s*\S+)?\s*&&\s+/;

function describeTool(name, raw, workspace) {
  const args = parseArgs(raw);
  if (!args) return { dir: "", text: raw };
  if (name === "bash") {
    const { dir, rest } = splitDirs(args.command || "");
    return { dir: shorten(dir, workspace), text: rest };
  }
  if (name === "read" || name === "write" || name === "edit") {
    return { dir: "", text: shorten(args.path || "", workspace) };
  }
  if (name === "search") return { dir: "", text: args.query || "" };
  if (name === "fetch") return { dir: "", text: args.url || "" };
  if (name === "browse") {
    return { dir: "", text: args.url || `${(args.steps || []).length} pasos` };
  }
  return { dir: "", text: JSON.stringify(args) };
}

function splitDirs(command) {
  const dirs = [];
  let rest = command.trim();
  let found = rest.match(CD);
  while (found) {
    dirs.push(found[1]);
    rest = rest.slice(found[0].length);
    found = rest.match(CD);
  }
  return { dir: dirs[dirs.length - 1] || "", rest };
}

function shorten(path, workspace) {
  if (!workspace) return path;
  if (path === workspace) return "~";
  if (path.startsWith(workspace + "/")) return path.slice(workspace.length + 1);
  return path;
}

function parseArgs(raw) {
  try {
    return JSON.parse(raw || "{}");
  } catch {
    return null;
  }
}

if (typeof module !== "undefined") module.exports = { describeTool, parseArgs };
