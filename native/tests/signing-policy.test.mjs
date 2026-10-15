import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (rel) => readFileSync(new URL(rel, root), "utf8");

const policy = JSON.parse(read("native/installer/signing/signing.json"));
const invoke = read("native/installer/signing/Invoke-CodeSigning.ps1");
const confirm = read("native/installer/signing/Confirm-ArtifactSignatures.ps1");
const workflow = read(".github/workflows/native-sign.yml");
const runbook = read("docs/NATIVE_CODE_SIGNING.md");
const iss = read("native/installer/remcord.iss");
const builder = read("native/installer/build-installer.ps1");

test("signing policy: versioned SHA256 policy with https timestamp", () => {
    assert.equal(policy.version, 1);
    assert.equal(policy.digestAlgorithm, "SHA256");
    assert.match(policy.timestampServer, /^https:\/\//);
    const host = new URL(policy.timestampServer).host;
    assert.ok(policy.allowedTimestampHosts.includes(host), "timestamp host must be allowlisted");
});

test("signing policy: exactly the two known relative artifacts", () => {
    assert.equal(policy.artifacts.length, 2);
    const paths = policy.artifacts.map((a) => a.path);
    assert.ok(paths.includes("native/installer/stage/remcord.exe"));
    const setup = policy.artifacts.find((a) => a.path.includes("Remcord-Setup-"));
    assert.ok(setup, "setup artifact pattern present");
    assert.ok(setup.path.includes("{version}"), "setup path carries the {version} placeholder");
    assert.equal(setup.versionSource, "package.json");
    for (const artifact of policy.artifacts) {
        assert.equal(artifact.required, true);
        assert.ok(!/^[A-Za-z]:[\\/]/.test(artifact.path), "no absolute artifact paths");
        assert.ok(!artifact.path.startsWith("\\\\"), "no UNC artifact paths");
    }
});

test("signing policy: SignPath-first providers resolve against declared secrets", () => {
    assert.deepEqual(policy.providers.order, ["signpath", "pfx"]);
    assert.ok(!("azureTrustedSigning" in policy.providers), "no Azure provider");
    const signpath = policy.providers.signpath;
    const pfx = policy.providers.pfx;
    const envNames = [
        signpath.tokenEnv,
        signpath.orgIdEnv,
        signpath.projectSlugEnv,
        signpath.policySlugEnv,
        pfx.base64Env,
        pfx.passwordEnv,
    ];
    for (const name of envNames) {
        assert.match(name, /^[A-Z][A-Z0-9_]+$/, `secret env name is SCREAMING_SNAKE_CASE: ${name}`);
        assert.ok(policy.secretNames.includes(name), `secret declared in secretNames: ${name}`);
    }
    assert.ok(Array.isArray(pfx.subjectAllowlist), "subject allowlist is an array");
    for (const subject of pfx.subjectAllowlist) {
        assert.match(subject, /^CN=/);
    }
});

test("signing policy: unsigned-by-default is explicit, flip is documented", () => {
    assert.equal(policy.releaseRequiresSignature, false);
    assert.ok(policy.releaseRequiresSignatureNote.includes("checklist D/F31"));
    assert.equal(typeof policy.allowUnsignedEnv, "string");
    assert.ok(policy.allowUnsignedEnv.length > 0);
});

test("installer definition stays signing-free (check-installer §7 contract)", () => {
    assert.ok(!/signtool/i.test(iss), "remcord.iss must not reference signtool");
    assert.ok(!/Invoke-CodeSigning/i.test(iss), "remcord.iss must not reference the signing step");
    assert.ok(
        builder.includes("No signing (separate blocker)"),
        "build-installer.ps1 still documents signing as a separate step",
    );
    assert.ok(!/Invoke-CodeSigning/i.test(builder), "build script never invokes signing itself");
});

test("invoke script: PFX-only, fail-closed, no URL key sources, no Azure", () => {
    assert.ok(invoke.includes("SupportsShouldProcess"), "dry-run support via ShouldProcess");
    assert.ok(invoke.includes("https?://"), "URL PFX sources are refused");
    assert.ok(invoke.includes("releaseRequiresSignature"), "policy gate honored");
    assert.ok(invoke.includes("allowedTimestampHosts"), "timestamp host allowlist enforced");
    assert.ok(invoke.includes("subjectAllowlist"), "subject pinning honored when populated");
    assert.ok(invoke.includes("WriteAllBytes"), "temp PFX is shredded after use");
    assert.ok(!/Invoke-WebRequest|curl\.exe|Start-BitsTransfer/.test(invoke), "no download machinery");
    assert.ok(!/AzureSignTool|azureTrustedSigning|AZURE_/i.test(invoke), "no Azure signing");
});

test("confirm script: read-only gate with local-only escape hatch", () => {
    assert.ok(confirm.includes("Get-AuthenticodeSignature"), "Authenticode verification present");
    assert.ok(confirm.includes("TimeStamperCertificate"), "RFC3161 timestamp presence checked");
    assert.ok(confirm.includes("GITHUB_ACTIONS"), "escape hatch disabled under CI");
    assert.ok(confirm.includes("RequireSigned"), "required mode supported");
    assert.ok(!/signtool sign|Set-AuthenticodeSignature/.test(confirm), "confirm script never signs");
});

test("workflow: signs Windows on release, SignPath-first, policy test first", () => {
    assert.ok(workflow.includes("workflow_dispatch"), "manual trigger present");
    assert.ok(workflow.includes("release"), "release trigger present");
    assert.ok(workflow.includes("windows-latest"), "Windows-only runner");
    assert.ok(workflow.includes("require-signed"), "fail-closed input present");
    assert.ok(workflow.includes("native/tests/signing-policy.test.mjs"), "policy test gates the job");
    assert.ok(workflow.includes("Confirm-ArtifactSignatures.ps1"), "audit step present on every run");
    assert.ok(workflow.includes("Invoke-CodeSigning.ps1"), "PFX fallback step present");
    assert.ok(/signpath/i.test(workflow), "SignPath step present");
    for (const secret of policy.secretNames) {
        assert.ok(workflow.includes(secret), `workflow wires secret ${secret}`);
    }
    assert.ok(!/AZURE_|AzureSignTool|azureTrustedSigning/i.test(workflow), "no Azure signing");
    assert.ok(!/^(\s*)needs:/m.test(workflow), "no job dependency on existing flows");
});

test("runbook: documents free SignPath path, secrets, and the signed-by-default flip", () => {
    assert.ok(!/Azure Trusted Signing/i.test(runbook), "no Azure option documented");
    assert.ok(/signpath/i.test(runbook), "SignPath free path documented");
    assert.ok(runbook.includes("SIGNPATH_API_TOKEN"), "SignPath secret documented");
    assert.ok(runbook.includes("WINDOWS_PFX_BASE64"), "PFX fallback secret documented");
    assert.ok(runbook.includes("never a release fix"), "self-signed scope is explicit");
    assert.ok(runbook.includes("releaseRequiresSignature"), "flip procedure references the policy flag");
    assert.ok(runbook.includes("SmartScreen"), "SmartScreen reality documented");
    for (const secret of policy.secretNames) {
        assert.ok(runbook.includes(secret), `runbook documents secret ${secret}`);
    }
});

test("scripts stay Windows PowerShell 5.1 safe: ASCII-only, no BOM dependence", () => {
    for (const [name, source] of [
        ["Invoke-CodeSigning.ps1", invoke],
        ["Confirm-ArtifactSignatures.ps1", confirm],
    ]) {
        assert.ok(
            /^[\x00-\x7F]*$/.test(source),
            `${name} must be ASCII-only (5.1 ParseFile misdetects BOM-less UTF-8 with non-ASCII)`,
        );
    }
});

test("no private key material in the new F31 files", () => {
    for (const [name, source] of [
        ["signing.json", JSON.stringify(policy)],
        ["Invoke-CodeSigning.ps1", invoke],
        ["Confirm-ArtifactSignatures.ps1", confirm],
        ["native-sign.yml", workflow],
        ["NATIVE_CODE_SIGNING.md", runbook],
    ]) {
        assert.ok(!/BEGIN (RSA )?PRIVATE KEY/.test(source), `${name}: no PEM private key`);
        assert.ok(!/base64,[A-Za-z0-9+/=]{500,}/.test(source), `${name}: no embedded base64 blob`);
    }
});
