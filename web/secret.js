const SECRET_COMMANDS = ["/secret", "/secreto"];
const SECRET_NAME = "SECRETO";

const SECRET_PREFIXES = [
  [/^sk-ant-/, "ANTHROPIC_API_KEY"],
  [/^sk-proj-/, "OPENAI_API_KEY"],
  [/^sk-/, "OPENAI_API_KEY"],
  [/^(ghp_|gho_|ghs_|ghu_|github_pat_)/, "GITHUB_TOKEN"],
  [/^xox[abposr]-/, "SLACK_TOKEN"],
  [/^(AKIA|ASIA)[0-9A-Z]{16}$/, "AWS_ACCESS_KEY_ID"],
  [/^AIza[0-9A-Za-z_-]{35}$/, "GOOGLE_API_KEY"],
  [/^hf_/, "HUGGING_FACE_TOKEN"],
  [/^-----BEGIN /, "PRIVATE_KEY"],
];

const SECRET_HEX = /^[0-9a-f]{40}$|^[0-9a-f]{64}$/;
const SECRET_BLOB = /^[A-Za-z0-9+/=_-]{32,}$/;

function secretName(value) {
  const text = String(value).trim();
  for (const [pattern, name] of SECRET_PREFIXES) {
    if (pattern.test(text)) return name;
  }
  return SECRET_NAME;
}

function secretShaped(value) {
  const text = String(value).trim();
  if (text.length < 16 || text.length > 4096) return false;
  if (/\s/.test(text)) return false;
  if (SECRET_PREFIXES.some(([pattern]) => pattern.test(text))) return true;
  if (SECRET_HEX.test(text)) return false;
  return SECRET_BLOB.test(text) && /\d/.test(text) && /[A-Za-z]/.test(text);
}

function secretKey(text) {
  return String(text)
    .trim()
    .toUpperCase()
    .replace(/[^A-Z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 64);
}

function secretCommand(text) {
  const trimmed = String(text).trim();
  const space = trimmed.indexOf(" ");
  const head = space < 0 ? trimmed : trimmed.slice(0, space);
  if (!SECRET_COMMANDS.includes(head)) return null;
  return { name: secretKey(space < 0 ? "" : trimmed.slice(space + 1)) };
}

function secretHint(value) {
  const text = String(value).trim();
  if (text.length <= 4) return "…";
  return "…" + text.slice(-4);
}

let secretCard = null;
let secretParts = null;

function secretElement(id) {
  return document.getElementById(id);
}

function secretOpen(name, value) {
  secretCard = { value: value || "" };
  secretElement("secret").hidden = false;
  secretElement("composer").classList.add("secreting");
  secretElement("secret-name").value = name || "";
  secretElement("secret-value").value = secretCard.value;
  secretElement("secret-scope").textContent = secretParts ? secretParts.scope() : "";
  secretElement("secret-hint").textContent = secretCard.value ? secretHint(secretCard.value) : "";
  secretParts.input.value = "";
  (name ? secretElement("secret-value") : secretElement("secret-name")).focus();
}

function secretClose() {
  secretCard = null;
  secretElement("secret").hidden = true;
  secretElement("composer").classList.remove("secreting");
  secretElement("secret-value").value = "";
  secretElement("secret-hint").textContent = "";
}

function secretShowing() {
  return secretCard !== null;
}

function secretMount(elements) {
  secretParts = elements;
  if (!secretElement("secret")) return;
  secretElement("secret-cancel").onclick = () => {
    secretClose();
    secretParts.input.focus();
  };
  secretElement("secret-save").onclick = () => secretSave();
  secretElement("secret-escape").onclick = () => {
    const value = secretCard ? secretCard.value || secretElement("secret-value").value : "";
    secretClose();
    secretParts.send(value);
  };
  secretElement("secret-name").addEventListener("input", (event) => {
    event.target.value = secretKey(event.target.value);
  });
  secretElement("secret").addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      secretClose();
      secretParts.input.focus();
    }
    if (event.key === "Enter") {
      event.preventDefault();
      secretSave();
    }
  });
}

async function secretSave() {
  if (!secretCard) return;
  const name = secretKey(secretElement("secret-name").value);
  const value = secretCard.value || secretElement("secret-value").value;
  if (!name) return secretParts.notify("poné un nombre para la variable");
  if (!value) return secretParts.notify("el valor está vacío");
  const saved = await api("/api/secret", {
    conversation: secretParts.conversation(),
    name,
    value,
  });
  if (!saved || !saved.saved) return secretParts.notify("no se guardó");
  secretClose();
  secretParts.notify(`🔑 ${saved.saved} quedó en el vault (${saved.project}/${saved.env})`);
  secretParts.input.focus();
}

if (typeof module !== "undefined") {
  module.exports = { secretName, secretShaped, secretKey, secretCommand, secretHint };
}
