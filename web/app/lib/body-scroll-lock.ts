interface ScrollLockTarget {
  style: {
    overflow: string;
  };
}

type ReleaseScrollLock = () => void;

export function createBodyScrollLockManager(
  resolveTarget: () => ScrollLockTarget | null,
): () => ReleaseScrollLock {
  let activeLocks = 0;
  let lockedTarget: ScrollLockTarget | null = null;
  let originalOverflow = "";

  return function acquireScrollLock() {
    if (activeLocks === 0) {
      const target = resolveTarget();
      if (!target) return () => {};
      lockedTarget = target;
      originalOverflow = target.style.overflow;
      target.style.overflow = "hidden";
    }

    activeLocks += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      activeLocks -= 1;
      if (activeLocks !== 0 || !lockedTarget) return;

      lockedTarget.style.overflow = originalOverflow;
      lockedTarget = null;
      originalOverflow = "";
    };
  };
}

const acquireDocumentBodyScrollLock = createBodyScrollLockManager(() => (
  typeof document === "undefined" ? null : document.body
));

export function acquireBodyScrollLock(): ReleaseScrollLock {
  return acquireDocumentBodyScrollLock();
}
