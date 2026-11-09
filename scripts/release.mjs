// The pushed tag starts the Build workflow, which makes the draft release.
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const sh = (cmd) => execSync(cmd, { encoding: "utf8" }).trim();
const fail = (msg) => {
  console.error(msg);
  process.exit(1);
};

const arg = process.argv[2];
if (!arg) fail("Usage: pnpm release <patch|minor|major|x.y.z>");

const pkg = JSON.parse(readFileSync("package.json", "utf8"));
const [major, minor, patch] = pkg.version.split(".").map(Number);
const next = {
  patch: `${major}.${minor}.${patch + 1}`,
  minor: `${major}.${minor + 1}.0`,
  major: `${major + 1}.0.0`,
}[arg] ?? arg;
if (!/^\d+\.\d+\.\d+$/.test(next)) fail(`Not a version: ${arg}`);
const tag = `v${next}`;

if (sh("git branch --show-current") !== "main") fail("Releases are cut from main.");
if (sh("git status --porcelain")) fail("Commit or stash your changes first.");
sh("git fetch --tags --quiet origin");
if (sh(`git tag --list ${tag}`)) fail(`${tag} already exists.`);
if (sh("git rev-list --count HEAD..origin/main") !== "0") fail("main is behind origin. Pull first.");

const replace = (file, pattern, value) => {
  const text = readFileSync(file, "utf8");
  if (!pattern.test(text)) fail(`Couldn't find the version in ${file}.`);
  writeFileSync(file, text.replace(pattern, value));
};
replace("package.json", /("version":\s*")[^"]+"/, `$1${next}"`);
replace("src-tauri/tauri.conf.json", /("version":\s*")[^"]+"/, `$1${next}"`);
replace("src-tauri/Cargo.toml", /^(version\s*=\s*")[^"]+"/m, `$1${next}"`);
// Otherwise the next cargo run rewrites Cargo.lock and leaves a stray change.
replace("src-tauri/Cargo.lock", /(name = "pipescope"\nversion = ")[^"]+"/, `$1${next}"`);

sh("git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock");
sh(`git commit --quiet -m "Release ${tag}"`);
sh(`git tag -a ${tag} -m "Pipescope ${tag}"`);
sh(`git push --quiet --atomic origin main ${tag}`);

console.log(`Released ${tag} (was v${pkg.version}). The Build workflow will attach it to a draft release.`);
