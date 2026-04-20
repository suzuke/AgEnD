import { join } from "node:path";

/**
 * Conservative whitelist for an instance name when it is going to be used as a
 * path segment. Allows letters, digits, `_`, `-`, `.`. Empty / pure-`.` /
 * pure-`..` is rejected so we cannot escape `dataDir/instances/`.
 *
 * Defence-in-depth: callers (CLI / fleet config loader) already constrain
 * instance names, but `resolveAccessPathFromConfig` is invoked from several
 * entry points and the consequence of a traversal here is reading or writing
 * an attacker-supplied file path.
 */
const VALID_INSTANCE_NAME = /^[A-Za-z0-9._-]+$/;

function assertSafeInstanceName(instance: string): void {
  if (!VALID_INSTANCE_NAME.test(instance) || instance === "." || instance === "..") {
    throw new Error(`Invalid instance name "${instance}" — must match ${VALID_INSTANCE_NAME}`);
  }
}

/**
 * Resolve the access.json path for an instance.
 * Topic mode uses fleet-level access; otherwise per-instance.
 *
 * Throws if `instance` is not a safe path segment (per-instance mode only;
 * topic mode does not embed `instance` in the returned path).
 */
export function resolveAccessPathFromConfig(
  dataDir: string,
  instance: string,
  fleetChannel: { mode?: string } | undefined,
): string {
  if (fleetChannel?.mode === "topic") {
    return join(dataDir, "access", "access.json");
  }
  assertSafeInstanceName(instance);
  return join(dataDir, "instances", instance, "access.json");
}
