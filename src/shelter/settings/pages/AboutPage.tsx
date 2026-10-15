import { createSignal, For, Show } from "solid-js";
import {
    copyAboutText,
    DISCORD_SERVER_INVITE,
    getAppVersionLabel,
    getRuntimeLabel,
    MAINTAINER_AVATAR_URL,
    MAINTAINER_DISCORD_USER_ID,
    MAINTAINER_FRIEND_URL,
    MAINTAINER_GITHUB_URL,
    MAINTAINER_GITHUB_USERNAME,
    MAINTAINER_SPONSOR_URL,
    openExternal,
} from "../about.js";
import { SettingsPageHeader } from "../components/SettingsPageHeader.jsx";
import classes from "./AboutPage.module.css";

const {
    ui: { Button, ButtonSizes, showToast },
} = shelter;

const TECHNOLOGIES = ["Electron", "Rust", "SolidJS", "TypeScript", "shelter", "Rolldown", "Biome"];

function ActionRow(props: { title: string; desc: string; icon: string; onClick: () => void; primary?: boolean }) {
    return (
        <button
            type="button"
            class={`${classes.action} ${props.primary ? classes.actionPrimary : ""}`}
            onClick={props.onClick}
        >
            <span class={classes.actionIcon} aria-hidden="true">
                {props.icon}
            </span>
            <span class={classes.actionLabel}>
                <span class={classes.actionTitle}>{props.title}</span>
                <span class={classes.actionDesc}>{props.desc}</span>
            </span>
            <svg
                class={classes.actionArrow}
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
    );
}

export function AboutPage() {
    const [avatarOk, setAvatarOk] = createSignal(true);
    const version = getAppVersionLabel();
    const runtime = getRuntimeLabel();

    function copyUserId() {
        void copyAboutText(MAINTAINER_DISCORD_USER_ID).then((ok) =>
            showToast({
                title: ok ? "Copied" : "Copy failed",
                content: ok
                    ? `Discord user ID ${MAINTAINER_DISCORD_USER_ID} copied to clipboard.`
                    : "Could not access the clipboard.",
                duration: 4000,
            }),
        );
    }

    return (
        <div class={classes.page}>
            <SettingsPageHeader title="About" description="Remcord, the people behind it, and where to find them." />

            <section class={classes.hero}>
                <div class={classes.wordmark}>REMCORD</div>
                <div class={classes.pills}>
                    <span class={classes.versionPill}>{version}</span>
                    <span class={classes.runtimePill}>{runtime}</span>
                </div>
                <p class={classes.tagline}>
                    A free and open-source Discord client crafted by the community, for the community — delivering
                    advanced features, better performance, and a modern design.
                </p>
            </section>

            <section class={classes.card}>
                <h3 class={classes.sectionTitle}>Maintainer</h3>
                <div class={classes.maintainer}>
                    <Show when={avatarOk()} fallback={<div class={classes.avatarFallback}>R</div>}>
                        <img
                            src={MAINTAINER_AVATAR_URL}
                            alt={`${MAINTAINER_GITHUB_USERNAME}'s avatar`}
                            class={classes.avatar}
                            loading="lazy"
                            onError={() => setAvatarOk(false)}
                        />
                    </Show>
                    <div class={classes.maintainerMeta}>
                        <div class={classes.maintainerName}>{MAINTAINER_GITHUB_USERNAME}</div>
                        <div class={classes.maintainerRole}>Creator &amp; maintainer of Remcord</div>
                        <p class={classes.maintainerBio}>
                            Builds Remcord in the open. Bug reports, ideas, and pull requests are always welcome — and
                            if Remcord makes your Discord experience better, consider sponsoring the work.
                        </p>
                    </div>
                </div>
                <div class={classes.maintainerButtons}>
                    <Button
                        size={ButtonSizes.LARGE}
                        class={classes.friendButton}
                        onClick={() => openExternal(MAINTAINER_FRIEND_URL)}
                    >
                        Add as Friend on Discord
                    </Button>
                    <button type="button" class={classes.copyId} onClick={copyUserId}>
                        Copy user ID: {MAINTAINER_DISCORD_USER_ID}
                    </button>
                </div>
            </section>

            <section class={classes.card}>
                <h3 class={classes.sectionTitle}>Connect</h3>
                <div class={classes.actions}>
                    <ActionRow
                        primary
                        title="Add as Friend"
                        desc="Open rawknee-69's Discord profile"
                        icon="✦"
                        onClick={() => openExternal(MAINTAINER_FRIEND_URL)}
                    />
                    <ActionRow
                        title="GitHub Profile"
                        desc={`@${MAINTAINER_GITHUB_USERNAME} on GitHub`}
                        icon="⌨"
                        onClick={() => openExternal(MAINTAINER_GITHUB_URL)}
                    />
                    <ActionRow
                        title="Sponsor"
                        desc="Support Remcord's development"
                        icon="♥"
                        onClick={() => openExternal(MAINTAINER_SPONSOR_URL)}
                    />
                    <ActionRow
                        title="Join Discord Server"
                        desc="Chat with the community"
                        icon="✉"
                        onClick={() => openExternal(DISCORD_SERVER_INVITE)}
                    />
                </div>
            </section>

            <section class={classes.card}>
                <h3 class={classes.sectionTitle}>Technologies</h3>
                <div class={classes.techList}>
                    <For each={TECHNOLOGIES}>
                        {(tech) => (
                            <span class={classes.techTag}>
                                {tech}
                                <Show when={tech === "Electron" && runtime.startsWith("Electron")}> · this build</Show>
                                <Show when={tech === "Rust" && runtime.startsWith("Native")}> · this build</Show>
                            </span>
                        )}
                    </For>
                </div>
            </section>

            <footer class={classes.footer}>
                <span>
                    Copyright &copy; Remcord 2024-2026 &middot; OSL-3.0 &middot; by {MAINTAINER_GITHUB_USERNAME}
                </span>
            </footer>
        </div>
    );
}
