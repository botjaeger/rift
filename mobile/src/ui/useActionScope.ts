import { useFocusEffect } from "expo-router";
import { useCallback, useState } from "react";

/** A captured action belongs to one visit, never a later refocus of the same screen. */
export function createActionScope() {
  let generation = 0;
  let active = true;
  return {
    capture() {
      const at = generation;
      return () => active && at === generation;
    },
    enter() { active = true; generation++; },
    leave() {
      if (!active) return false;
      active = false;
      generation++;
      return true;
    },
  };
}

export function useActionScope(identity = "") {
  const [scope] = useState(createActionScope);
  useFocusEffect(useCallback(() => {
    scope.enter();
    return () => { scope.leave(); };
  // A route parameter change is a new visit even when navigation reuses its component.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, identity]));
  return scope;
}
