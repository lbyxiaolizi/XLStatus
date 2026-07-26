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

import { ReactNode, useCallback, useId, useRef, useState } from "react";
import { buttonClass, inputClass, textareaClass, useDialogFocusTrap } from "./M7Primitives";
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

type DialogRequest =
  | { kind: "confirm"; opts: ConfirmOptions }
  | { kind: "prompt"; opts: PromptOptions }
  | { kind: "totp"; opts: TotpOptions };
type ActiveDialog = DialogRequest & { id: number };

export interface Dialogs {
  confirm: (opts: ConfirmOptions) => Promise<boolean>;
  prompt: (opts: PromptOptions) => Promise<string | null>;
  totp: (opts?: TotpOptions) => Promise<string | null>;
  element: ReactNode;
}

function dialogCancellationValue(kind: DialogRequest["kind"] | null): false | null {
  return kind === "confirm" ? false : null;
}

function Backdrop({
  children,
  titleId,
  descriptionId,
  onClose,
}: {
  children: ReactNode;
  titleId: string;
  descriptionId?: string;
  onClose: () => void;
}) {
  const dialogRef = useDialogFocusTrap(onClose);

  return (
    <div className="fixed inset-0 z-[60] flex items-start justify-center overflow-y-auto bg-black/50 px-4 py-4 sm:py-16">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        tabIndex={-1}
        className="w-full max-w-md border-4 border-black bg-[var(--bg-card)] shadow-[10px_10px_0_0_#000] outline-none"
      >
        {children}
      </div>
    </div>
  );
}

function DialogHeader({ id, title }: { id: string; title: string }) {
  return (
    <div className="border-b-4 border-black bg-[var(--accent-bg)] px-5 py-4">
      <h2 id={id} className="break-words text-lg font-black uppercase text-[var(--text-main)]">{title}</h2>
    </div>
  );
}

export function useDialogs(): Dialogs {
  const [active, setActive] = useState<ActiveDialog | null>(null);
  const resolveRef = useRef<((value: unknown) => void) | null>(null);
  const activeKindRef = useRef<DialogRequest["kind"] | null>(null);
  const nextDialogIdRef = useRef(0);
  const [value, setValue] = useState("");
  const [fieldError, setFieldError] = useState<string | null>(null);
  const titleId = useId();
  const descriptionId = useId();
  const fieldId = useId();
  const errorId = useId();

  const settle = useCallback((result: unknown) => {
    const resolve = resolveRef.current;
    resolveRef.current = null;
    activeKindRef.current = null;
    setActive(null);
    setValue("");
    setFieldError(null);
    resolve?.(result);
  }, []);

  const open = useCallback((dialog: DialogRequest, initial = "") => {
    // If a dialog is somehow already open, cancel it first.
    const previousResolve = resolveRef.current;
    const previousKind = activeKindRef.current;
    resolveRef.current = null;
    activeKindRef.current = null;
    previousResolve?.(dialogCancellationValue(previousKind));
    nextDialogIdRef.current += 1;
    const id = nextDialogIdRef.current;
    return new Promise<unknown>((resolve) => {
      resolveRef.current = resolve;
      activeKindRef.current = dialog.kind;
      setValue(initial);
      setFieldError(null);
      setActive({ ...dialog, id });
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
    const dismiss = () => settle(dialogCancellationValue(active.kind));

    if (active.kind === "confirm") {
      const { opts } = active;
      element = (
        <Backdrop key={active.id} titleId={titleId} descriptionId={descriptionId} onClose={dismiss}>
          <DialogHeader id={titleId} title={opts.title ?? copy.dialogs.pleaseConfirm} />
          <div className="space-y-5 px-5 py-5">
            <div id={descriptionId} className="text-sm font-bold text-[var(--text-main)]">{opts.message}</div>
            <div className="flex flex-wrap justify-end gap-2">
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => settle(false)}
                data-dialog-initial-focus
              >
                {opts.cancelLabel ?? cancelText}
              </button>
              <button
                type="button"
                className={buttonClass(opts.danger ? "danger" : "primary")}
                onClick={() => settle(true)}
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
        <Backdrop
          key={active.id}
          titleId={titleId}
          descriptionId={opts.message ? descriptionId : undefined}
          onClose={dismiss}
        >
          <DialogHeader id={titleId} title={opts.title ?? opts.label ?? copy.dialogs.pleaseInput} />
          <form
            className="space-y-5 px-5 py-5"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            {opts.message ? (
              <div id={descriptionId} className="text-sm font-bold text-[var(--text-main)]">{opts.message}</div>
            ) : null}
            {opts.label ? (
              <label htmlFor={fieldId} className="block text-xs font-black uppercase tracking-wide text-[var(--text-main)]">
                {opts.label}
              </label>
            ) : null}
            {opts.multiline ? (
              <textarea
                id={fieldId}
                className={textareaClass}
                rows={4}
                value={value}
                placeholder={opts.placeholder}
                aria-label={opts.label ? undefined : opts.title ?? copy.dialogs.pleaseInput}
                aria-invalid={Boolean(fieldError)}
                aria-describedby={fieldError ? errorId : undefined}
                onChange={(event) => {
                  setValue(event.target.value);
                  if (fieldError) setFieldError(null);
                }}
                data-dialog-initial-focus
              />
            ) : (
              <input
                id={fieldId}
                className={inputClass}
                value={value}
                placeholder={opts.placeholder}
                aria-label={opts.label ? undefined : opts.title ?? copy.dialogs.pleaseInput}
                aria-invalid={Boolean(fieldError)}
                aria-describedby={fieldError ? errorId : undefined}
                onChange={(event) => {
                  setValue(event.target.value);
                  if (fieldError) setFieldError(null);
                }}
                data-dialog-initial-focus
              />
            )}
            {fieldError ? (
              <span id={errorId} role="alert" className="block text-xs font-bold text-[var(--accent-color)]">{fieldError}</span>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
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
        <Backdrop key={active.id} titleId={titleId} descriptionId={descriptionId} onClose={dismiss}>
          <DialogHeader id={titleId} title={opts.title ?? copy.dialogs.twoFactor} />
          <form
            className="space-y-5 px-5 py-5"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            <label id={descriptionId} htmlFor={fieldId} className="text-sm font-bold text-[var(--text-main)]">
              {opts.message ?? copy.dialogs.totpPrompt}
            </label>
            <input
              id={fieldId}
              className={`${inputClass} text-center text-2xl tracking-[0.5em]`}
              value={code}
              inputMode="numeric"
              autoComplete="one-time-code"
              maxLength={6}
              placeholder="000000"
              aria-invalid={Boolean(fieldError)}
              aria-describedby={fieldError ? errorId : undefined}
              onChange={(event) => {
                setValue(event.target.value.replace(/\D/g, "").slice(0, 6));
                if (fieldError) setFieldError(null);
              }}
              data-dialog-initial-focus
            />
            {fieldError ? (
              <span id={errorId} role="alert" className="block text-xs font-bold text-[var(--accent-color)]">{fieldError}</span>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
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
