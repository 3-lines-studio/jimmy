const test = require("node:test");
const assert = require("node:assert");
const { secretName, secretShaped, secretKey, secretCommand, secretHint } = require("./secret.js");

test("una clave conocida adivina el nombre de la variable", () => {
  assert.equal(secretName("sk-ant-api03-abcdefghijklmnop"), "ANTHROPIC_API_KEY");
  assert.equal(secretName("sk-proj-abcdefghijklmnop"), "OPENAI_API_KEY");
  assert.equal(secretName("ghp_abcdefghijklmnopqrstuvwxyz0123"), "GITHUB_TOKEN");
  assert.equal(secretName("github_pat_11ABCDEFG0123456789"), "GITHUB_TOKEN");
  assert.equal(secretName("xoxb-1234-5678-abcdefghij"), "SLACK_TOKEN");
  assert.equal(secretName("AKIAABCDEFGHIJKLMNOP"), "AWS_ACCESS_KEY_ID");
  assert.equal(secretName("AIzaSyA1234567890abcdefghijklmnopqrstuv"), "GOOGLE_API_KEY");
  assert.equal(secretName("-----BEGIN RSA PRIVATE KEY-----"), "PRIVATE_KEY");
  assert.equal(secretName("a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6"), "SECRETO");
});

test("pegar prosa o una URL no es pegar un secreto", () => {
  assert.equal(secretShaped("che, mirá esto"), false);
  assert.equal(secretShaped("https://example.com/a/b?c=1"), false);
  assert.equal(secretShaped("corta"), false);
  assert.equal(secretShaped("4f9a1c2b7d8e3f5a6b0c1d2e3f4a5b6c7d8e9f01"), false);
  assert.equal(secretShaped("a".repeat(5000)), false);
});

test("un token opaco se reconoce por su forma", () => {
  assert.equal(secretShaped("sk-abcdefghijklmnop1234"), true);
  assert.equal(secretShaped("ghp_abcdefghijklmnopqrstuvwxyz0123"), true);
  assert.equal(secretShaped("Zx9kQ2mW7pL4nR8tV6yB3cF1hJ5sD0gA"), true);
  assert.equal(secretShaped("  sk-abcdefghijklmnop1234  "), true);
});

test("el nombre se normaliza a mayúsculas y guiones bajos", () => {
  assert.equal(secretKey("openai api-key"), "OPENAI_API_KEY");
  assert.equal(secretKey("  ya está  "), "YA_EST");
  assert.equal(secretKey("a".repeat(80)).length, 64);
});

test("el comando pide el nombre y no el valor", () => {
  assert.deepEqual(secretCommand("/secret OPENAI_API_KEY"), { name: "OPENAI_API_KEY" });
  assert.deepEqual(secretCommand("/secreto"), { name: "" });
  assert.deepEqual(secretCommand("  /secret   github token  "), { name: "GITHUB_TOKEN" });
  assert.equal(secretCommand("/secrets OPENAI_API_KEY"), null);
  assert.equal(secretCommand("mirá /secret"), null);
  assert.equal(secretCommand(""), null);
});

test("la pista muestra sólo el final", () => {
  assert.equal(secretHint("sk-abcdefghijklmnop1234"), "…1234");
  assert.equal(secretHint("abc"), "…");
});
