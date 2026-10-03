function projectHue(name) {
  let hash = 0;
  for (const character of name) hash = (hash * 31 + character.codePointAt(0)) % 360;
  return hash;
}

function projectColor(name) {
  return `hsl(${projectHue(name)} 55% 42%)`;
}

const PROJECT_BODIES = ["round", "squircle", "square", "egg"];
const PROJECT_EYES = ["bar", "dot", "square", "slit"];

function projectHash(name, seed) {
  let hash = (2166136261 ^ seed) >>> 0;
  for (const character of name) {
    hash ^= character.codePointAt(0);
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  hash ^= hash >>> 15;
  hash = Math.imul(hash, 2246822507) >>> 0;
  return (hash ^ (hash >>> 13)) >>> 0;
}

function projectFace(name) {
  return {
    body: PROJECT_BODIES[projectHash(name, 7) % PROJECT_BODIES.length],
    eyes: PROJECT_EYES[projectHash(name, 53) % PROJECT_EYES.length],
  };
}

function projectInitials(name) {
  return name.slice(0, 3);
}

function projectTitle(name) {
  return name.slice(0, 1).toUpperCase() + name.slice(1);
}

function projectSize(bytes) {
  const kb = 1000;
  const mb = kb * 1000;
  const gb = mb * 1000;
  if (bytes >= gb)
    return (bytes / gb).toFixed(1).replace(".", ",").replace(",0", "") + " GB";
  if (bytes >= mb) return Math.round(bytes / mb) + " MB";
  return Math.round(bytes / kb) + " KB";
}

if (typeof module !== "undefined")
  module.exports = {
    PROJECT_BODIES,
    PROJECT_EYES,
    projectColor,
    projectFace,
    projectHue,
    projectInitials,
    projectTitle,
    projectSize,
  };
