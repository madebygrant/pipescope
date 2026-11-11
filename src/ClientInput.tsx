import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { resolveClient } from "./model";

interface Props {
  label: string;
  // The repo's current client, or "" for none.
  value: string;
  names: string[];
  className?: string;
  // Only called when the client actually changes; null clears it.
  onSave: (client: string | null) => void;
  onClose: () => void;
}

const MAX_SHOWN = 8;
// Roughly MAX_SHOWN rows; below this much room the list opens upwards instead.
const LIST_HEIGHT = 180;

export function ClientInput({ label, value, names, className, onSave, onClose }: Props) {
  const listId = useId();
  const [text, setText] = useState(value);
  const [open, setOpen] = useState(true);
  // The box opens holding the current client; filtering on that would hide the others.
  const [edited, setEdited] = useState(false);
  const [active, setActive] = useState(-1);
  const [above, setAbove] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  // Enter and Escape unmount the input, and the blur that follows mustn't act a second time.
  const done = useRef(false);

  const query = edited ? text.trim().toLowerCase() : "";
  const allMatches = names.filter((n) => n.toLowerCase().includes(query));
  const matches = allMatches.slice(0, MAX_SHOWN);
  const more = allMatches.length - matches.length;
  const showList = open && matches.length > 0;

  // Measured against the scrolling list, not the window: the hint footer sits below it.
  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!showList || !input) return;
    const scroller = input.closest(".list");
    const measure = () => {
      const box = input.getBoundingClientRect();
      const area = scroller?.getBoundingClientRect() ?? { top: 0, bottom: window.innerHeight };
      setAbove(area.bottom - box.bottom < LIST_HEIGHT && box.top - area.top > LIST_HEIGHT);
    };
    measure();
    scroller?.addEventListener("scroll", measure);
    return () => scroller?.removeEventListener("scroll", measure);
  }, [showList]);

  useEffect(() => {
    if (active >= 0) listRef.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const finish = (raw: string) => {
    if (done.current) return;
    done.current = true;
    const next = resolveClient(raw, names, value);
    onClose();
    if (next !== undefined) onSave(next);
  };

  const cancel = () => {
    done.current = true;
    onClose();
  };

  return (
    <span className={`client-combo${className ? ` ${className}` : ""}`}>
      <input
        ref={inputRef}
        className="client-input"
        autoFocus
        value={text}
        placeholder="Client, blank for none"
        aria-label={label}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={showList}
        aria-controls={showList ? listId : undefined}
        aria-activedescendant={showList && active >= 0 ? `${listId}-${active}` : undefined}
        onFocus={(e) => e.currentTarget.select()}
        onChange={(e) => {
          setText(e.target.value);
          setEdited(true);
          setOpen(true);
          setActive(-1);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            // Arrows here move through suggestions, not the run list behind.
            e.stopPropagation();
            setOpen(true);
            const last = matches.length - 1;
            setActive((i) => (e.key === "ArrowDown" ? Math.min(i + 1, last) : Math.max(i - 1, -1)));
          } else if (e.key === "Enter") {
            e.preventDefault();
            // The names can change under an open list, so fall back to the typed text.
            finish((showList && matches[active]) || text);
          } else if (e.key === "Tab" && showList && matches[active]) {
            finish(matches[active]);
          } else if (e.key === "Escape") {
            // Stops the app's window-level Escape from also leaving the view.
            e.stopPropagation();
            if (showList) setOpen(false);
            else cancel();
          }
        }}
        onBlur={() => finish(text)}
      />
      {showList && (
        <ul ref={listRef} id={listId} role="listbox" aria-label="Clients" className={`client-options${above ? " above" : ""}`}>
          {matches.map((name, i) => (
            <li
              key={name}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              className={`client-option${i === active ? " active" : ""}`}
              // Keeps focus in the input, so picking doesn't blur and save the typed text first.
              onMouseDown={(e) => e.preventDefault()}
              onMouseMove={() => setActive(i)}
              onClick={() => finish(name)}
            >
              {name}
            </li>
          ))}
          {more > 0 && (
            <li className="client-more" aria-hidden="true">
              +{more} more, keep typing
            </li>
          )}
        </ul>
      )}
    </span>
  );
}
