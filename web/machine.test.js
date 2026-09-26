const test = require("node:test");
const assert = require("node:assert");
const { machineSize, machineRatio, machineDetail } = require("./machine.js");

test("los megabytes van sin decimales y los gigas con uno", () => {
  assert.equal(machineSize(29 * 1000 * 1000), "29 MB");
  assert.equal(machineSize(1000 * 1000 * 1000), "1 GB");
  assert.equal(machineSize(1.6 * 1000 * 1000 * 1000), "1,6 GB");
});

test("un uso se muestra contra su tope", () => {
  assert.equal(
    machineRatio(1.9 * 1000 * 1000 * 1000, 32 * 1000 * 1000 * 1000),
    "1,9 GB / 32 GB",
  );
  assert.equal(
    machineRatio(2.2 * 1000 * 1000 * 1000, 4.6 * 1000 * 1000 * 1000),
    "2,2 GB / 4,6 GB",
  );
});

test("el desglose separa lo mío del cache y del kernel", () => {
  const machine = {
    memory: {
      used: 2 * 1000 * 1000 * 1000,
      total: 32 * 1000 * 1000 * 1000,
      anon: 30 * 1000 * 1000,
      cache: 1.6 * 1000 * 1000 * 1000,
      kernel: 274 * 1000 * 1000,
    },
    disk: { used: 1, total: 2 },
    processes: 7,
  };
  assert.equal(
    machineDetail(machine),
    "anon 30 MB · cache 1,6 GB · kernel 274 MB · 7 procesos",
  );
});
