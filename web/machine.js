const GB = 1000 * 1000 * 1000;
const MB = 1000 * 1000;

function machineSize(bytes) {
  if (bytes >= GB) return decimal(bytes / GB) + " GB";
  return Math.round(bytes / MB) + " MB";
}

function machineRatio(used, total) {
  return machineSize(used) + " / " + machineSize(total);
}

function machineDetail(machine) {
  const memory = machine.memory;
  return [
    "anon " + machineSize(memory.anon),
    "cache " + machineSize(memory.cache),
    "kernel " + machineSize(memory.kernel),
    machine.processes + " procesos",
  ].join(" · ");
}

function decimal(value) {
  return value.toFixed(1).replace(".", ",").replace(",0", "");
}

if (typeof module !== "undefined") {
  module.exports = { machineSize, machineRatio, machineDetail };
}
