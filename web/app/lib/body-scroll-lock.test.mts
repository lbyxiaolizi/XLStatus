import assert from "node:assert/strict";
import { test } from "node:test";
import { createBodyScrollLockManager } from "./body-scroll-lock.ts";

function createTarget(overflow = "") {
  return { style: { overflow } };
}

test("nested locks restore the original overflow after either release order", () => {
  for (const releaseOuterFirst of [true, false]) {
    const target = createTarget("clip");
    const acquire = createBodyScrollLockManager(() => target);
    const releaseOuter = acquire();
    const releaseInner = acquire();

    assert.equal(target.style.overflow, "hidden");
    if (releaseOuterFirst) releaseOuter();
    else releaseInner();
    assert.equal(target.style.overflow, "hidden");

    if (releaseOuterFirst) releaseInner();
    else releaseOuter();
    assert.equal(target.style.overflow, "clip");
  }
});

test("release functions are idempotent", () => {
  const target = createTarget("auto");
  const acquire = createBodyScrollLockManager(() => target);
  const release = acquire();

  release();
  release();
  assert.equal(target.style.overflow, "auto");

  const releaseAgain = acquire();
  assert.equal(target.style.overflow, "hidden");
  releaseAgain();
  assert.equal(target.style.overflow, "auto");
});

test("a missing target is a no-op", () => {
  const acquire = createBodyScrollLockManager(() => null);
  assert.doesNotThrow(() => acquire()());
});
