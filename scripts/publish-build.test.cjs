const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const publish = require("./publish-build.cjs");

function fixture() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "peng-release-"));
  fs.writeFileSync(
    path.join(directory, "app-debug.apk"),
    Buffer.from("PK-android-fixture"),
  );
  fs.writeFileSync(
    path.join(directory, "PengMiner_1.0.0_x64-setup.exe"),
    Buffer.from("MZ-windows-fixture"),
  );
  return directory;
}
function api(releases = []) {
  const assets = [];
  const calls = [];
  const repos = {
    listReleases: () => {},
    createRelease: async (value) => {
      calls.push(["draft", value]);
      return { data: { id: 12, assets: [] } };
    },
    uploadReleaseAsset: async (value) => {
      assets.push({
        name: value.name,
        state: "uploaded",
        size: value.data.length,
      });
      calls.push(["asset", value]);
    },
    listReleaseAssets: async () => ({ data: assets }),
    updateRelease: async (value) => {
      calls.push(["publish", value]);
      return { data: { html_url: "https://example.com/release" } };
    },
  };
  return {
    github: { rest: { repos }, paginate: async () => releases },
    assets,
    calls,
  };
}
const context = {
  repo: { owner: "owner", repo: "repository" },
  runId: 123,
  sha: "tested-commit",
};
const core = {
  notice: () => {},
  summary: {
    addLink() {
      return this;
    },
    async write() {},
  },
};

test("publishes both builds and checksums only after all uploads exist", async () => {
  const directory = fixture();
  try {
    const fake = api();
    await publish({ github: fake.github, context, core, directory });
    assert.equal(fake.calls[0][0], "draft");
    assert.equal(fake.calls[0][1].tag_name, "build-123");
    assert.equal(fake.calls[0][1].target_commitish, "tested-commit");
    assert.equal(fake.calls[0][1].draft, true);
    assert.deepEqual(
      fake.assets.map((asset) => asset.name).sort(),
      [
        "PengMiner-Android-debug.apk",
        "PengMiner_1.0.0_x64-setup.exe",
        "SHA256SUMS.txt",
      ].sort(),
    );
    const last = fake.calls.at(-1);
    assert.equal(last[0], "publish");
    assert.equal(last[1].draft, false);
  } finally {
    fs.rmSync(directory, { recursive: true });
  }
});
test("does not publish an incomplete artifact set", async () => {
  const directory = fixture();
  try {
    fs.unlinkSync(path.join(directory, "app-debug.apk"));
    const fake = api();
    await assert.rejects(
      publish({ github: fake.github, context, core, directory }),
      /Both Android and Windows/,
    );
    assert.equal(fake.calls.length, 0);
  } finally {
    fs.rmSync(directory, { recursive: true });
  }
});
test("refuses to overwrite a release from another commit", async () => {
  const directory = fixture();
  try {
    const fake = api([
      {
        tag_name: "build-123",
        target_commitish: "another-commit",
        draft: false,
      },
    ]);
    await assert.rejects(
      publish({ github: fake.github, context, core, directory }),
      /commit differs/,
    );
    assert.equal(fake.calls.length, 0);
  } finally {
    fs.rmSync(directory, { recursive: true });
  }
});
