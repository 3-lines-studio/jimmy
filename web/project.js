function projectHue(name) {
  let hash = 0;
  for (const character of name) hash = (hash * 31 + character.codePointAt(0)) % 360;
  return hash;
}

function projectColor(name) {
  return `hsl(${projectHue(name)} 55% 42%)`;
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
    projectColor,
    projectHue,
    projectInitials,
    projectTitle,
    projectSize,
  };
