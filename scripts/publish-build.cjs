const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");

function collect(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    return entry.isDirectory() ? collect(file) : [file];
  });
}

module.exports = async ({
  github,
  context,
  core,
  directory = "release-assets",
}) => {
  const files = collect(directory)
    .filter((file) => /\.(apk|exe)$/i.test(file))
    .map((file) => {
      let name = path.basename(file);
      if (name === "app-debug.apk") name = "PengMiner-Android-debug.apk";
      if (name === "app-release.apk") name = "PengMiner-Android.apk";
      return { name, data: fs.readFileSync(file) };
    });
  if (
    !files.some((file) => file.name.endsWith(".apk")) ||
    !files.some((file) => file.name.endsWith(".exe"))
  ) {
    throw new Error("Both Android and Windows build artifacts are required");
  }
  if (new Set(files.map((file) => file.name)).size !== files.length)
    throw new Error("Duplicate asset names");
  for (const file of files) {
    const header = file.data.subarray(0, 2).toString("hex");
    if (header !== (file.name.endsWith(".apk") ? "504b" : "4d5a"))
      throw new Error("Invalid binary artifact");
  }
  files.sort((a, b) => a.name.localeCompare(b.name));
  const hash = (data) => crypto.createHash("sha256").update(data).digest("hex");
  files.push({
    name: "SHA256SUMS.txt",
    data: Buffer.from(
      files.map((file) => `${hash(file.data)}  ${file.name}\n`).join(""),
    ),
  });
  const tag = `build-${context.runId}`;
  const releases = await github.paginate(
    github.rest.repos.listReleases,
    context.repo,
  );
  let release = releases.find((item) => item.tag_name === tag);
  if (release && release.target_commitish !== context.sha)
    throw new Error("Release commit differs");
  if (release && !release.draft) {
    const { data: existing } = await github.rest.repos.listReleaseAssets({
      ...context.repo,
      release_id: release.id,
    });
    if (
      !files.every((file) =>
        existing.some(
          (asset) =>
            asset.name === file.name &&
            asset.digest === `sha256:${hash(file.data)}`,
        ),
      )
    ) {
      throw new Error(
        "Published release assets differ; refusing to replace an existing release",
      );
    }
    core.notice(`Release ${tag} already published with matching assets`);
    return;
  }
  const debug = files.some((file) => file.name.includes("debug"));
  const body =
    `PengMiner from commit \`${context.sha}\`.\n\n` +
    `Automatic release after server, frontend, CLI, real mining integration, Android, and Windows installer checks pass.\n\n` +
    `Downloads: Android APK, Windows installer, and SHA-256 checksums.\n\n` +
    `Android uses the desktop interface with native CPU mining and Android Keystore login storage. ` +
    `Persistent login requires the updated PengChain server and remains valid until sign-out or revocation.\n\n` +
    (debug ? `The Android APK is debug-signed for sideloading.\n\n` : "") +
    `[Build and tests](https://github.com/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}).`;
  if (!release) {
    ({ data: release } = await github.rest.repos.createRelease({
      ...context.repo,
      tag_name: tag,
      target_commitish: context.sha,
      name: tag,
      body,
      draft: true,
    }));
  }
  for (const file of files) {
    const prior = release.assets.find((asset) => asset.name === file.name);
    if (prior)
      await github.rest.repos.deleteReleaseAsset({
        ...context.repo,
        asset_id: prior.id,
      });
    await github.rest.repos.uploadReleaseAsset({
      ...context.repo,
      release_id: release.id,
      name: file.name,
      data: file.data,
      headers: {
        "content-type": "application/octet-stream",
        "content-length": file.data.length,
      },
    });
  }
  const { data: assets } = await github.rest.repos.listReleaseAssets({
    ...context.repo,
    release_id: release.id,
  });
  if (
    !files.every((file) =>
      assets.some(
        (asset) =>
          asset.name === file.name &&
          asset.state === "uploaded" &&
          asset.size === file.data.length,
      ),
    )
  ) {
    throw new Error("Incomplete release upload; keeping release in draft");
  }
  const { data: published } = await github.rest.repos.updateRelease({
    ...context.repo,
    release_id: release.id,
    name: tag,
    body,
    draft: false,
    make_latest: "true",
  });
  await core.summary.addLink("Published release", published.html_url).write();
};
