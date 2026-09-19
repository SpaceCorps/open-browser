import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** The same `cn` as `@spacecorps/components`. Duplicated rather than imported: that package is
 * private and open-browser is public, so a dependency on it would make this repo unbuildable for
 * anyone outside the org. See `apps/web/README.md`. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
