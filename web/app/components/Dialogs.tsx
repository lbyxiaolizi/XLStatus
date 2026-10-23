"use client";

import { ReactNode, useCallback, useId, useRef, useState } from "react";
import { buttonClass, inputClass, textareaClass, useDialogFocusTrap } from "./M7Primitives";
import { getTranslations } from "@/lib/i18n";

export interface ConfirmOptions {
  title?: string | undefined;
  message: ReactNode;
  confirmLabel?: string | undefined;
  cancelLabel?: string | undefined;
  danger?: boolean | undefined;
}

export interface PromptOptions {
  title?: string | undefined;
  message?: ReactNode | undefined;
  label?: string | undefined;
  placeholder?: string | undefined;
  initialValue?: string | undefined;
  confirmLabel?: string | undefined;
  cancelLabel?: string | undefined;
  multiline?: boolean | undefined;
  validate?: ((value: string) => string | null | undefined) | undefined;
}

export interface TotpOptions {
  title?: string | undefined;
  message?: ReactNode | undefined;
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
  descriptionId?: string | undefined;
  onClose: () => void;
}) {
  const dialogRef = useDialogFocusTrap(onClose);

  return (
    <div className="fixed inset-0 z-[60] flex items-start justify-center overflow-y-auto bg-black/55 backdrop-blur-xs px-4 py-6 sm:py-16">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        tabIndex={-1}
        className="w-full max-w-md overflow-hidden rounded-2xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-lg)] outline-none"
      >
        {children}
      </div>
    </div>
  );
}

function DialogHeader({ id, title }: { id: string; title: string }) {
  return (
    <div className="border-b border-[var(--border-color)] bg-[var(--bg-subtle)] px-5 py-3.5">
      <h2 id={id} className="break-words text-base font-semibold tracking-tight text-[var(--text-main)]">{title}</h2>
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
            <div id={descriptionId} className="text-sm text-[var(--text-main)]">{opts.message}</div>
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
            className="space-y-4 px-5 py-5"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            {opts.message ? (
              <div id={descriptionId} className="text-sm text-[var(--text-main)]">{opts.message}</div>
            ) : null}
            {opts.label ? (
              <label htmlFor={fieldId} className="block text-xs font-medium text-[var(--text-main)]">
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
              <span id={errorId} role="alert" className="block text-xs font-medium text-[var(--status-danger-text)]">{fieldError}</span>
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
            className="space-y-4 px-5 py-5"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            <label id={descriptionId} htmlFor={fieldId} className="block text-sm text-[var(--text-main)]">
              {opts.message ?? copy.dialogs.totpPrompt}
            </label>
            <input
              id={fieldId}
              className={`${inputClass} text-center font-mono text-2xl tracking-[0.45em]`}
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
              <span id={errorId} role="alert" className="block text-xs font-medium text-[var(--status-danger-text)]">{fieldError}</span>
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
