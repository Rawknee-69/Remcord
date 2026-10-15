import { createSignal, For } from "solid-js";
import classes from "./SupportBanner.module.css";

const {
    ui: { ModalRoot, ModalBody, ModalSizes },
} = shelter;

interface Contributor {
    login: string;
    id: number;
    avatar_url: string;
    html_url: string;
    contributions: number;
    type: string;
}

/** Neutral state when no Remcord-owned contributor source is configured. */
export const CONTRIBUTORS_UNCONFIGURED_MESSAGE = "Contributor information is not configured for this build.";

export const AboutPopup = (props: { close: () => void }) => {
    // No Remcord-owned contributor source exists yet, so contributor data is
    // explicitly unavailable in this build. This screen performs no network
    // request and never attributes another product's contributors to Remcord.
    const [contributors] = createSignal<Contributor[]>([]);
    const [loading] = createSignal(false);
    const [error] = createSignal<string | null>(CONTRIBUTORS_UNCONFIGURED_MESSAGE);

    return (
        <ModalRoot size={ModalSizes.MEDIUM} class={classes.modal}>
            <div className={classes.closeButtonWrapper}>
                <button
                    type="button"
                    className={classes.closeButton}
                    onClick={(e) => {
                        e.stopPropagation();
                        props.close();
                    }}
                    aria-label="Close"
                >
                    <svg
                        width="20"
                        height="20"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                    >
                        <line x1="18" y1="6" x2="6" y2="18" />
                        <line x1="6" y1="6" x2="18" y2="18" />
                    </svg>
                </button>
            </div>
            <ModalBody>
                <div class={classes.aboutContainer}>
                    <div class={classes.aboutHeader}>
                        <div class={classes.aboutLogo}>
                            <span class={classes.aboutWordmark}>REMCORD</span>
                        </div>
                        <span class={classes.aboutVersion}>
                            {window.remcord?.version === "0.0.0"
                                ? "Dev Build"
                                : `v${window.remcord?.version ?? "1.3.0"}`}
                        </span>
                        <p class={classes.aboutDescription}>
                            A free and open-source Discord client crafted by the community, for the community —
                            delivering advanced features, better performance, and a modern design.
                        </p>
                    </div>

                    <div class={classes.quickActions}>
                        <h3 class={classes.sectionTitle}>Quick Actions</h3>
                        <div class={classes.quickActionsList}>
                            <button
                                type="button"
                                class={classes.quickActionButton}
                                onClick={() => window.open("https://github.com/sponsors/rawknee-69", "_blank")}
                            >
                                <span class={classes.quickActionButtonIcon}>
                                    <svg
                                        width="20"
                                        height="20"
                                        viewBox="0 0 24 24"
                                        fill="none"
                                        stroke="currentColor"
                                        stroke-width="2"
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                        aria-hidden="true"
                                    >
                                        <line x1="12" y1="1" x2="12" y2="23" />
                                        <path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6" />
                                    </svg>
                                </span>
                                <span class={classes.quickActionButtonLabel}>
                                    <span class={classes.quickActionButtonTitle}>Donate</span>
                                    <span class={classes.quickActionButtonDesc}>Support the project</span>
                                </span>
                                <svg
                                    class={classes.quickActionButtonArrow}
                                    width="16"
                                    height="16"
                                    viewBox="0 0 24 24"
                                    fill="none"
                                    stroke="currentColor"
                                    stroke-width="2"
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    aria-hidden="true"
                                >
                                    <polyline points="9 18 15 12 9 6" />
                                </svg>
                            </button>
                        </div>
                    </div>

                    <div class={classes.contributorsSection}>
                        <h3 class={classes.sectionTitle}>Contributors</h3>
                        {!loading() && !error() && (
                            <span class={classes.contributorBadge}>{contributors().length} contributors</span>
                        )}
                        {loading() && <div class={classes.loadingState}>Loading contributors...</div>}
                        {error() && <div class={classes.errorState}>{error()}</div>}
                        {!loading() && !error() && (
                            <div class={classes.contributorsGrid}>
                                <For each={contributors()}>
                                    {(contributor) => (
                                        <a
                                            href={contributor.html_url}
                                            target="_blank"
                                            rel="noopener noreferrer"
                                            class={classes.contributorLink}
                                        >
                                            <div class={classes.contributorCard}>
                                                <img
                                                    src={contributor.avatar_url}
                                                    alt={`${contributor.login}'s avatar`}
                                                    class={classes.contributorAvatar}
                                                />
                                                <div class={classes.contributorName}>{contributor.login}</div>
                                                <div class={classes.contributorInfo}>
                                                    {contributor.contributions} contributions
                                                </div>
                                            </div>
                                        </a>
                                    )}
                                </For>
                            </div>
                        )}
                    </div>

                    <div class={classes.techSection}>
                        <h3 class={classes.sectionTitle}>Technologies</h3>
                        <div class={classes.techList}>
                            <span class={classes.techTag}>Electron</span>
                            <span class={classes.techTag}>SolidJS</span>
                            <span class={classes.techTag}>TypeScript</span>
                            <span class={classes.techTag}>Rolldown</span>
                            <span class={classes.techTag}>shelter</span>
                            <span class={classes.techTag}>Biome</span>
                        </div>
                    </div>

                    <div class={classes.aboutFooter}>
                        <span>Copyright &copy; Remcord 2024-2026 &middot; OSL-3.0</span>
                    </div>
                </div>
            </ModalBody>
        </ModalRoot>
    );
};
