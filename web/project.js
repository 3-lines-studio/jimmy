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

function projectLetter(name) {
  return name.slice(0, 1).toUpperCase();
}

function projectTitle(name) {
  return name.slice(0, 1).toUpperCase() + name.slice(1);
}

if (typeof module !== "undefined")
  module.exports = { projectColor, projectInitials, projectLetter, projectTitle };
