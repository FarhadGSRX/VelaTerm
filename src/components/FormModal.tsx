//! Generic form modal that renders inputs from field definitions for creating groups/sessions, renaming, and similar actions.

import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { normalizeArgDashes } from "../args";
import { useT } from "../i18n";
import { Backdrop } from "./Backdrop";
import { Field, FieldStack } from "./Field";
import Select from "./Select";

export interface FormRenderContext {
  values: Record<string, string>;
  changeValues: (changes: Record<string, string>) => void;
  disabled: boolean;
}

export interface FieldDef {
  key: string;
  label: string;
  placeholder?: string;
  render?: (value: string, onChange: (value: string) => void, context: FormRenderContext) => ReactNode;
  required?: boolean;
  autoFocus?: boolean;
  /** When provided, render a select whose option value is submitted; an empty string is a valid Default option. */
  select?: { value: string; label: string }[];
  /** In select mode, append a Custom… option that reveals a text field; values outside the options enter this mode automatically. */
  allowCustom?: boolean;
  /** Text input by default; checkbox renders a toggle and hidden retains a dependent draft value. */
  type?: "text" | "checkbox" | "hidden";
  /** Value written when a checkbox is checked (default "1"), mapping binary semantics to a concrete string such as "skip". */
  checkedValue?: string;
  /** Value written when a checkbox is unchecked (default ""). */
  uncheckedValue?: string;
  /** Optional supporting text below a checkbox. */
  hint?: string;
  /** For text fields, restore `—`/`–` to `--` on submit so macOS Smart Punctuation cannot corrupt launch arguments. */
  normalizeDashes?: boolean;
}

/** Sentinel value for Custom… in a <select>; it cannot collide with a real shell path. */
const CUSTOM_SENTINEL = "\u0000__custom__";

/** Select field with options and an optional Custom… text-entry fallback. */
function SelectField({
  field,
  value,
  onChange,
  disabled,
}: {
  field: FieldDef;
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  const t = useT();
  const opts = field.select ?? [];
  const known = opts.some((o) => o.value === value);
  // A non-empty initial value outside the options enters custom mode and populates the text field.
  const [custom, setCustom] = useState(field.allowCustom === true && !known && value !== "");

  return (
    <>
      <Select
        disabled={disabled}
        value={custom ? CUSTOM_SENTINEL : value}
        onChange={(v) => {
          if (v === CUSTOM_SENTINEL) {
            setCustom(true);
          } else {
            setCustom(false);
            onChange(v);
          }
        }}
        options={[
          ...opts,
          ...(field.allowCustom
            ? [{ value: CUSTOM_SENTINEL, label: t("form.customOption"), separatorBefore: true }]
            : []),
        ]}
        width="100%"
        ariaLabel={field.label}
      />
      {custom && (
        <input
          className="vlx-input"
          disabled={disabled}
          style={{ marginTop: 6 }}
          autoCapitalize="none"
          placeholder={field.placeholder}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
    </>
  );
}

export function FormModal({
  title,
  description,
  fields,
  initial,
  submitLabel,
  submittingLabel,
  onSubmit,
  onCancel,
  onCancelSubmit,
  validate,
  formatSubmitError,
  onValuesChange,
}: {
  title: string;
  description?: string;
  fields: FieldDef[];
  initial?: Record<string, string>;
  submitLabel?: string;
  submittingLabel?: string;
  onSubmit: (values: Record<string, string>) => void | Promise<void>;
  onCancel: () => void;
  /** Cancel an in-flight operation before closing; omitted operations remain non-dismissible. */
  onCancelSubmit?: () => Promise<void>;
  /** Returns an error message to show below the fields and block submission, or null when the values are valid. */
  validate?: (values: Record<string, string>) => string | null;
  /** Formats submission failures for display; the default preserves the error message. */
  formatSubmitError?: (error: unknown) => string;
  /** Persist presentation drafts, for example in a recoverable dialog URL. */
  onValuesChange?: (values: Record<string, string>) => void;
}) {
  const t = useT();
  const dialogRef = useRef<HTMLDivElement>(null);
  const descriptionId = useId();
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = dialogRef.current;
    if (dialog && !dialog.contains(document.activeElement)) dialog.focus();
    return () => { if (previous?.isConnected) previous.focus(); };
  }, []);
  const [values, setValues] = useState<Record<string, string>>(() => {
    const v: Record<string, string> = {};
    for (const f of fields) v[f.key] = initial?.[f.key] ?? "";
    return v;
  });
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const [cancelling, setCancelling] = useState(false);
  const cancellingRef = useRef(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const changeValues = (changes: Record<string, string>) => {
    if (submittingRef.current || cancellingRef.current) return;
    setSubmitError(null);
    const next = { ...values, ...changes };
    setValues(next);
    onValuesChange?.(next);
  };
  const changeValue = (key: string, value: string) => changeValues({ [key]: value });
  const cancel = () => {
    if (cancellingRef.current) return;
    if (!submittingRef.current) { onCancel(); return; }
    if (!onCancelSubmit) return;
    cancellingRef.current = true;
    setCancelling(true);
    setSubmitError(null);
    void onCancelSubmit().then(onCancel).catch(e => {
      setSubmitError(formatSubmitError ? formatSubmitError(e) : e instanceof Error ? e.message : String(e));
    }).finally(() => {
      cancellingRef.current = false;
      setCancelling(false);
    });
  };

  const validationError = validate?.(values) ?? null;
  const error = validationError ?? submitError;
  const busy = submitting || cancelling;
  const canSubmit = !busy && !validationError && fields.every(
    (f) => !f.required || values[f.key].trim().length > 0,
  );

  const submit = async () => {
    if (!canSubmit || submittingRef.current || cancellingRef.current) return;
    // On submit, restore long dashes to `--` in fields marked normalizeDashes, such as launch arguments.
    const out: Record<string, string> = { ...values };
    for (const f of fields) {
      if (f.normalizeDashes) out[f.key] = normalizeArgDashes(out[f.key] ?? "");
    }
    submittingRef.current = true;
    setSubmitting(true);
    setSubmitError(null);
    try {
      await onSubmit(out);
    } catch (e) {
      if (!cancellingRef.current) setSubmitError(formatSubmitError ? formatSubmitError(e) : e instanceof Error ? e.message : String(e));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  };

  return (
    <Backdrop onClose={cancel}>
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        aria-describedby={description ? descriptionId : undefined}
        aria-busy={busy}
        style={{
          width: 380,
          maxWidth: "calc(100vw - 32px)",
          maxHeight: "calc(100dvh - 32px)",
          overflowY: "auto",
          outline: "none",
          background: "var(--bg-panel)",
          border: "1px solid var(--border)",
          borderRadius: 10,
          padding: 18,
          boxShadow: "0 10px 40px rgba(0,0,0,0.5)",
        }}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.defaultPrevented) return;
          if (e.key === "Enter" && e.target instanceof HTMLButtonElement) return;
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void submit();
          }
          if (e.key === "Escape") cancel();
        }}
      >
        <div
          style={{
            fontSize: 14,
            fontWeight: 600,
            color: "var(--text-primary)",
            marginBottom: 14,
          }}
        >
          {title}
        </div>

        {description && <p id={descriptionId} style={{ color: "var(--text-secondary)", fontSize: 12, lineHeight: 1.6, margin: "0 0 14px" }}>
          {description}
        </p>}

        <FieldStack>
          {fields.map((f) => {
            if (f.type === "hidden") return null;
            if (f.type === "checkbox") {
              const on = f.checkedValue ?? "1";
              const checked = values[f.key] === on;
              return (
                <div key={f.key}>
                  <label
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                      fontSize: 13,
                      color: "var(--text-primary)",
                      cursor: "pointer",
                    }}
                  >
                    <input
                      type="checkbox"
                      disabled={busy}
                      checked={checked}
                      onChange={(e) =>
                        changeValue(f.key, e.target.checked ? on : f.uncheckedValue ?? "")
                      }
                    />
                    {f.label}
                  </label>
                  {f.hint && (
                    <div
                      style={{
                        fontSize: 11,
                        color: "var(--text-muted)",
                        lineHeight: 1.5,
                        marginTop: 4,
                        marginLeft: 24,
                      }}
                    >
                      {f.hint}
                    </div>
                  )}
                </div>
              );
            }
            if (f.render) return <Field key={f.key} as="div" label={f.label} required={f.required}>
              {f.render(values[f.key], v => changeValue(f.key, v), { values, changeValues, disabled: busy })}
            </Field>;
            return (
              <Field key={f.key} label={f.label} required={f.required}>
                {f.select ? (
                  <SelectField
                    field={f}
                    value={values[f.key]}
                    disabled={busy}
                    onChange={(v) => changeValue(f.key, v)}
                  />
                ) : (
                <input
                  className="vlx-input"
                  disabled={busy}
                  autoCapitalize="none"
                  placeholder={f.placeholder}
                  autoFocus={f.autoFocus}
                  value={values[f.key]}
                  onChange={(e) =>
                    changeValue(f.key, e.target.value)
                  }
                />
                )}
              </Field>
            );
          })}
        </FieldStack>

        {error && (
          <div role="alert" style={{ fontSize: 12, color: "var(--status-error)", marginTop: 10 }}>
            {error}
          </div>
        )}

        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: 8,
            marginTop: 18,
          }}
        >
          <button className="vlx-btn" onClick={cancel} disabled={cancelling || (submitting && !onCancelSubmit)}>
            {t("common.cancel")}
          </button>
          <button
            className="vlx-btn vlx-btn-primary"
            onClick={() => void submit()}
            disabled={!canSubmit}
          >
            {submitting && submittingLabel ? <span role="status">{submittingLabel}</span> : submitLabel ?? t("common.confirm")}
          </button>
        </div>
      </div>
    </Backdrop>
  );
}
