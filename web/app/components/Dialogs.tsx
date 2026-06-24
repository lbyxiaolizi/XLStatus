"use client";

// In-app dialog primitives replacing the native window.confirm / window.prompt
// calls scattered across the dashboard. The native dialogs freeze the JS
// thread, can't be styled or validated inline, and are blocked in some
// embedded/mobile webviews — the main source of the "awkward interaction"
// complaint.
//
// Usage (per page, no provider needed):
//   const dialogs = useDialogs();
//   ...
//   if (!(await dialogs.confirm({ message: "删除该服务器？", danger: true }))) return;
//   const name = await dialogs.prompt({ label: "组名" });   // string | null
//   const code = await dialogs.totp();                      // string | null
//   ...
//   return ( <> ...page... {dialogs.element} </> );
//
// Each call resolves and closes immediately; in-flight "busy" state belongs on
// the page's own action button, not the dialog.

import { ReactNode, useCallback, useRef, useState } from "react";
import { buttonClass, inputClass, textareaClass } from "./M7Primitives";
import { getTranslations } from "@/lib/i18n";

export interface ConfirmOptions {
  title?: string;
  message: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
}

export interface PromptOptions {
  title?: string;
  message?: ReactNode;
  label?: string;
  placeholder?: string;
  initialValue?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  multiline?: boolean;
  // Return an error string to block submission, or null/undefined to allow.
  validate?: (value: string) => string | null | undefined;
}

export interface TotpOptions {
  title?: string;
  message?: ReactNode;
}

type ActiveDialog =
  | { kind: "confirm"; opts: ConfirmOptions }
  | { kind: "prompt"; opts: PromptOptions }
  | { kind: "totp"; opts: TotpOptions };

export interface Dialogs {
  confirm: (opts: ConfirmOptions) => Promise<boolean>;
  prompt: (opts: PromptOptions) => Promise<string | null>;
  totp: (opts?: TotpOptions) => Promise<string | null>;
  element: ReactNode;
}

function Backdrop({ children }: { children: ReactNode }) {
  return (
    <div className="fixed inset-0 z-[60] flex items-start justify-center overflow-y-auto bg-black/50 px-4 py-16">
      <div className="w-full max-w-md border-4 border-black bg-[var(--bg-card)] shadow-[10px_10px_0_0_#000]">
        {children}
      </div>
    </div>
  );
}

function DialogHeader({ title }: { title: string }) {
  return (
    <div className="border-b-4 border-black bg-[var(--accent-bg)] px-5 py-4">
      <h2 className="text-lg font-black uppercase text-[var(--text-main)]">{title}</h2>
    </div>
  );
}

export function useDialogs(): Dialogs {
  const [active, setActive] = useState<ActiveDialog | null>(null);
  const resolveRef = useRef<((value: unknown) => void) | null>(null);
  const [value, setValue] = useState("");
  const [fieldError, setFieldError] = useState<string | null>(null);

  const settle = useCallback((result: unknown) => {
    const resolve = resolveRef.current;
    resolveRef.current = null;
    setActive(null);
    setValue("");
    setFieldError(null);
    resolve?.(result);
  }, []);

  const open = useCallback((dialog: ActiveDialog, initial = "") => {
    // If a dialog is somehow already open, cancel it first.
    resolveRef.current?.(dialog.kind === "confirm" ? false : null);
    return new Promise<unknown>((resolve) => {
      resolveRef.current = resolve;
      setValue(initial);
      setFieldError(null);
      setActive(dialog);
    });
  }, []);

  const confirm = useCallback(
    (opts: ConfirmOptions) => open({ kind: "confirm", opts }) as Promise<boolean>,
    [open],
  );
  const prompt = useCallback(
    (opts: PromptOptions) =>
      open({ kind: "prompt", opts }, opts.initialValue ?? "") as Promise<string | null>,
    [open],
  );
  const totp = useCallback(
    (opts: TotpOptions = {}) => open({ kind: "totp", opts }) as Promise<string | null>,
    [open],
  );

  let element: ReactNode = null;
  if (active) {
    const copy = getTranslations();
    const cancelText = copy.common.cancel;

    if (active.kind === "confirm") {
      const { opts } = active;
      element = (
        <Backdrop>
          <DialogHeader title={opts.title ?? copy.dialogs.pleaseConfirm} />
          <div className="space-y-5 px-5 py-5">
            <div className="text-sm font-bold text-[var(--text-main)]">{opts.message}</div>
            <div className="flex justify-end gap-2">
              <button type="button" className={buttonClass("secondary")} onClick={() => settle(false)}>
                {opts.cancelLabel ?? cancelText}
              </button>
              <button
                type="button"
                className={buttonClass(opts.danger ? "danger" : "primary")}
                onClick={() => settle(true)}
                autoFocus
              >
                {opts.confirmLabel ?? copy.dialogs.confirm}
              </button>
            </div>
          </div>
        </Backdrop>
      );
    } else if (active.kind === "prompt") {
      const { opts } = active;
      const submit = () => {
        const error = opts.validate?.(value);
        if (error) {
          setFieldError(error);
          return;
        }
        settle(value);
      };
      element = (
        <Backdrop>
          <DialogHeader title={opts.title ?? opts.label ?? copy.dialogs.pleaseInput} />
          <form
            className="space-y-5 px-5 py-5"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            {opts.message ? (
              <div className="text-sm font-bold text-[var(--text-main)]">{opts.message}</div>
            ) : null}
            {opts.label ? (
              <span className="block text-xs font-black uppercase tracking-wide text-[var(--text-main)]">
                {opts.label}
              </span>
            ) : null}
            {opts.multiline ? (
              <textarea
                className={textareaClass}
                rows={4}
                value={value}
                placeholder={opts.placeholder}
                onChange={(event) => {
                  setValue(event.target.value);
                  if (fieldError) setFieldError(null);
                }}
                autoFocus
              />
            ) : (
              <input
                className={inputClass}
                value={value}
                placeholder={opts.placeholder}
                onChange={(event) => {
                  setValue(event.target.value);
                  if (fieldError) setFieldError(null);
                }}
                autoFocus
              />
            )}
            {fieldError ? (
              <span className="block text-xs font-bold text-[var(--accent-color)]">{fieldError}</span>
            ) : null}
            <div className="flex justify-end gap-2">
              <button type="button" className={buttonClass("secondary")} onClick={() => settle(null)}>
                {opts.cancelLabel ?? cancelText}
              </button>
              <button type="submit" className={buttonClass("primary")}>
                {opts.confirmLabel ?? copy.dialogs.ok}
              </button>
            </div>
          </form>
        </Backdrop>
      );
    } else {
      const { opts } = active;
      const code = value;
      const submit = () => {
        if (!/^\d{6}$/.test(code)) {
          setFieldError(getTranslations().dialogs.totpInvalid);
          return;
        }
        settle(code);
      };
      element = (
        <Backdrop>
          <DialogHeader title={opts.title ?? copy.dialogs.twoFactor} />
          <form
            className="space-y-5 px-5 py-5"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            <div className="text-sm font-bold text-[var(--text-main)]">
              {opts.message ?? copy.dialogs.totpPrompt}
            </div>
            <input
              className={`${inputClass} text-center text-2xl tracking-[0.5em]`}
              value={code}
              inputMode="numeric"
              autoComplete="one-time-code"
              maxLength={6}
              placeholder="000000"
              onChange={(event) => {
                setValue(event.target.value.replace(/\D/g, "").slice(0, 6));
                if (fieldError) setFieldError(null);
              }}
              autoFocus
            />
            {fieldError ? (
              <span className="block text-xs font-bold text-[var(--accent-color)]">{fieldError}</span>
            ) : null}
            <div className="flex justify-end gap-2">
              <button type="button" className={buttonClass("secondary")} onClick={() => settle(null)}>
                {cancelText}
              </button>
              <button type="submit" className={buttonClass("primary")}>
                {copy.dialogs.ok}
              </button>
            </div>
          </form>
        </Backdrop>
      );
    }
  }

  return { confirm, prompt, totp, element };
}
