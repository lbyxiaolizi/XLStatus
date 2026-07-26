"use client";

import { useEffect, useRef, type RefObject } from "react";

// Cards that own their data loading attach this ref to their outermost node so
// the request only fires when the card is about to enter the viewport. The
// callback runs at most once per mount, even under StrictMode double effects.
// Environments without IntersectionObserver (old webviews, jsdom) load eagerly.
export function useLoadOnVisible<T extends HTMLElement = HTMLDivElement>(
  load: () => void,
): RefObject<T | null> {
  const targetRef = useRef<T | null>(null);
  const loadRef = useRef(load);
  const loadedRef = useRef(false);

  useEffect(() => {
    loadRef.current = load;
  }, [load]);

  useEffect(() => {
    function runOnce() {
      if (loadedRef.current) return;
      loadedRef.current = true;
      loadRef.current();
    }

    const target = targetRef.current;
    if (!target || typeof IntersectionObserver === "undefined") {
      runOnce();
      return;
    }

    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        observer.unobserve(entry.target);
        runOnce();
      }
    }, { rootMargin: "400px 0px" });
    observer.observe(target);
    return () => observer.disconnect();
  }, []);

  return targetRef;
}
