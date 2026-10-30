// A made-up inventory of about 2.7 MB, the size of the "2.7 MB JSON" case in docs/design.md#speed,
// built in the page so the speed can be tried without a download. One host repeats its "port"
// key. Names are under example.com and addresses in the documentation ranges of RFC 5737.

const ROLES = ["web", "api", "db", "cache", "queue", "search", "mail"];
const REGIONS = ["eu-west", "eu-north", "us-east", "us-west", "ap-south"];
const NETS = ["192.0.2.", "198.51.100.", "203.0.113."];
export const HOSTS = 12_660;

function host(i, twice) {
  const role = ROLES[i % ROLES.length];
  const region = REGIONS[(i * 3) % REGIONS.length];
  const name = `${role}-${String(i + 1).padStart(5, "0")}`;
  const port = 8000 + (i % 9) * 100 + 43;
  const extra = twice ? `, "port": ${port + 1000}` : "";
  return (
    `    {"id": ${i + 1}, "name": "${name}", "fqdn": "${name}.${region}.example.com", ` +
    `"ip": "${NETS[i % 3]}${(i % 254) + 1}", "port": ${port}, "region": "${region}", ` +
    `"weight": ${((i * 37) % 100) / 100}, "healthy": ${i % 11 !== 0}${extra}, ` +
    `"tags": ["${role}", "${region}", "rack-${(i % 40) + 1}"]}`
  );
}

export function hosts() {
  const dup = Math.floor(HOSTS * 0.62);
  const rows = Array.from({ length: HOSTS }, (_, i) => host(i, i === dup));
  return `{\n  "inventory": "fleet.example.com",\n  "hosts": [\n${rows.join(",\n")}\n  ]\n}\n`;
}
