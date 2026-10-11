/**
 * Release-metadata source configuration for `scripts/utils/updateMeta.ts`.
 *
 * There is no default owner/repository: the script must fail closed with a
 * useful message unless `REMCORD_META_REPO` explicitly names a
 * Remcord-owned `owner/repo` source.
 */

export interface MetaRepo {
    owner: string;
    repo: string;
}

const REPO_PATTERN = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

/**
 * Resolve the metadata source from the environment.
 * Throws a descriptive error when unconfigured or malformed — callers must
 * surface it and exit non-zero without issuing any network request.
 */
export function resolveMetaRepo(env: NodeJS.ProcessEnv = process.env): MetaRepo {
    const raw = env.REMCORD_META_REPO?.trim() ?? "";
    if (raw.length === 0) {
        throw new Error(
            "REMCORD_META_REPO is not configured. Set REMCORD_META_REPO to the Remcord-owned " +
                '"owner/repo" that publishes releases before running updateMeta.',
        );
    }
    if (!REPO_PATTERN.test(raw)) {
        throw new Error(
            `REMCORD_META_REPO must be "owner/repo", got ${JSON.stringify(raw)}. ` +
                "Set it to the Remcord-owned repository that publishes releases.",
        );
    }
    const [owner, repo] = raw.split("/");
    return { owner, repo };
}

/** Release API endpoint for an explicitly configured metadata source. */
export function metaReleasesUrl(source: MetaRepo): string {
    return `https://api.github.com/repos/${source.owner}/${source.repo}/releases/latest`;
}
