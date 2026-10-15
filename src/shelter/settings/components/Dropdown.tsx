import { createEffect, createMemo, createSignal, For, onCleanup, onMount } from "solid-js";
import classes from "./Dropdown.module.css";

export const Dropdown = (props: {
    value: string | number;
    onChange: (val: string) => void;
    options: { label: string; value: string | number }[];
    styles?: { [key: string]: string };
    limitHeight?: boolean | undefined;
    maxHeight?: number;
    class?: string;
    disabled?: boolean | undefined;
}) => {
    const [open, set] = createSignal(false);
    const [maxHeight, setMaxHeight] = createSignal(
        typeof props.maxHeight === "number" ? `${props.maxHeight}px` : "240px",
    );
    let container: HTMLDivElement | undefined;

    const close = () => set(false);

    const handler = (e: MouseEvent) => !container?.contains(e.target as Node) && close();

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            close();
        }
    };

    onMount(() => {
        document.addEventListener("click", handler);
        document.addEventListener("keydown", onKeyDown);
    });
    onCleanup(() => {
        document.removeEventListener("click", handler);
        document.removeEventListener("keydown", onKeyDown);
    });

    createEffect(() => {
        if (!open()) return;
        if (typeof props.maxHeight === "number") {
            setMaxHeight(`${props.maxHeight}px`);
            return;
        }
        // Long lists (tray has 10 entries) must scroll instead of growing
        // past the settings pane and appearing blank/clipped.
        if (!props.limitHeight) {
            setMaxHeight("240px");
            return;
        }

        const rect = container?.parentElement?.getBoundingClientRect();
        if (!rect) return;

        const availableSpace = rect.bottom - container!.getBoundingClientRect().bottom - 20;
        setMaxHeight(`${Math.max(120, Math.min(280, availableSpace))}px`);
    });

    const text = createMemo(() => props.options.find((o) => o.value === props.value)?.label ?? props.value);

    const select = (val: string | number) => {
        props.onChange(String(val));
        close();
    };

    return (
        <div ref={container} class={`${classes.container} ${props.class ?? ""}`.trim()} style={props.styles?.container}>
            <button
                type="button"
                class={classes.valuewrapper}
                disabled={props.disabled}
                onClick={() => {
                    if (!props.disabled) set(!open());
                }}
                style={props.styles?.valuewrapper}
            >
                <div class={classes.value} data-text-variant="text-md/medium" style={props.styles?.value}>
                    {text()}
                </div>
                <div>
                    <svg
                        aria-hidden="true"
                        role="img"
                        xmlns="http://www.w3.org/2000/svg"
                        width="18"
                        height="18"
                        fill="none"
                        viewBox="0 0 24 24"
                    >
                        <path
                            fill="currentColor"
                            d="M5.3 9.3a1 1 0 0 1 1.4 0l5.3 5.29 5.3-5.3a1 1 0 1 1 1.4 1.42l-6 6a1 1 0 0 1-1.4 0l-6-6a1 1 0 0 1 0-1.42Z"
                        />
                    </svg>
                </div>
            </button>
            {open() && (
                <div class={classes.list} role="listbox" style={{ "max-height": maxHeight() }}>
                    <For each={props.options}>
                        {(opt) => (
                            <div
                                role="option"
                                data-value={opt.value}
                                aria-selected={opt.value === props.value}
                                tabindex={0}
                                onMouseDown={(e) => {
                                    // Select on mousedown so the document click
                                    // closer cannot swallow the click first.
                                    e.preventDefault();
                                    select(opt.value);
                                }}
                                onClick={(e) => {
                                    e.stopPropagation();
                                    select(opt.value);
                                }}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter" || e.key === " ") {
                                        e.preventDefault();
                                        select(opt.value);
                                    }
                                }}
                            >
                                {opt.label}
                            </div>
                        )}
                    </For>
                </div>
            )}
        </div>
    );
};
