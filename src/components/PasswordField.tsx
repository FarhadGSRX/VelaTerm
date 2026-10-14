//! Password input with a trailing reveal toggle, shared by every panel that asks for a password.
//! The revealed state is local and resets whenever the field unmounts, so a password is never left
//! visible after the panel closes.

import { useState, type ReactNode } from "react";
import { useT } from "../i18n";

/** Default box styling; callers may override or extend it through `inputStyle`. */
const baseInputStyle: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "8px 10px",
  border: "1px solid var(--border)",
  borderRadius: "var(--r-sm, 6px)",
  background: "var(--bg-0)",
  color: "var(--text)",
  fontSize: 13,
  outline: "none",
};

export function PasswordField({
  value,
  placeholder,
  autoFocus,
  disabled,
  ariaLabel,
  onChange,
  onKeyDown,
  wrapStyle,
  inputStyle,
  actions,
  showLabel,
  hideLabel,
}: {
  value: string;
  placeholder?: string;
  autoFocus?: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  onChange: (v: string) => void;
  onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
  wrapStyle?: React.CSSProperties;
  inputStyle?: React.CSSProperties;
  /** Optional action next to the reveal button, such as copying the current value. */
  actions?: ReactNode;
  showLabel?: string;
  hideLabel?: string;
}) {
  const t = useT();
  const [shown, setShown] = useState(false);
  const revealLabel = shown ? hideLabel ?? t("connect.hidePassword") : showLabel ?? t("connect.showPassword");
  return (
    <div style={{ position: "relative", ...wrapStyle }}>
      <input
        type={shown ? "text" : "password"}
        value={value}
        placeholder={placeholder}
        autoFocus={autoFocus}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        style={{ ...baseInputStyle, ...inputStyle, paddingRight: actions ? 64 : 32 }}
      />
      {actions && <div style={{ position: "absolute", top: 0, bottom: 0, right: 30, display: "flex", alignItems: "center" }}>{actions}</div>}
      <button
        type="button"
        disabled={disabled}
        onClick={() => setShown((v) => !v)}
        title={revealLabel}
        aria-label={revealLabel}
        style={{
          position: "absolute",
          top: 0,
          bottom: 0,
          right: 0,
          width: 30,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          border: "none",
          background: "transparent",
          padding: 0,
          color: "var(--text-dim)",
          cursor: "pointer",
        }}
      >
        {shown ? <EyeOffGlyph /> : <EyeGlyph />}
      </button>
    </div>
  );
}

/** 14px Lucide eye icon, inheriting currentColor. */
function EyeGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

/** 14px Lucide eye-off icon, inheriting currentColor. */
function EyeOffGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M10.7 5.1A9.9 9.9 0 0 1 12 5c6.5 0 10 7 10 7a17.6 17.6 0 0 1-3 3.9" />
      <path d="M6.6 6.6A17.6 17.6 0 0 0 2 12s3.5 7 10 7a9.7 9.7 0 0 0 5.4-1.6" />
      <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2" />
      <path d="m2 2 20 20" />
    </svg>
  );
}

export default PasswordField;
