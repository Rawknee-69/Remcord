import classes from "./DropdownItem.module.css";

const {
    ui: { Divider, Header, HeaderTags, TextBox },
} = shelter;

export const TextBoxItem = (props: {
    title: string;
    note: string;
    value: string;
    onInput: (v: string) => void;
    /** Native shell: unsupported settings render disabled with a reason in `note`. */
    disabled?: boolean | undefined;
}) => {
    return (
        <div class={classes.item} style={props.disabled ? { opacity: "0.55" } : undefined}>
            <Header class={classes.title} tag={HeaderTags.H3}>
                {props.title}
            </Header>
            <div class={classes.note}>{props.note}</div>
            <br />
            <TextBox value={props.value} onInput={props.onInput} disabled={props.disabled} />
            <Divider />
        </div>
    );
};
