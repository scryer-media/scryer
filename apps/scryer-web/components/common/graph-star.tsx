import { createLucideIcon } from "lucide-react";

/**
 * Lucide graph-star: https://github.com/lucide-icons/lucide/blob/main/icons/graph-star.svg
 * ISC License
 * Copyright (c) 2026 Lucide Icons and Contributors
 *
 * Permission to use, copy, modify, and/or distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
export const GraphStar = createLucideIcon("GraphStar", [
  ["path", { d: "m12.342 9.316 1.764-3.527", key: "edge-top" }],
  ["path", { d: "m12.59 14.545 2.35 3.759", key: "edge-bottom" }],
  ["path", { d: "m13.982 12.332 4.03.447", key: "edge-right" }],
  ["path", { d: "M5.414 17.586 8.88 14.12", key: "edge-bottom-left" }],
  ["path", { d: "M9.025 9.742 5.317 5.505", key: "edge-top-left" }],
  ["circle", { cx: "11", cy: "12", r: "3", key: "center" }],
  ["circle", { cx: "15", cy: "4", r: "2", key: "top" }],
  ["circle", { cx: "16", cy: "20", r: "2", key: "bottom" }],
  ["circle", { cx: "20", cy: "13", r: "2", key: "right" }],
  ["circle", { cx: "4", cy: "19", r: "2", key: "bottom-left" }],
  ["circle", { cx: "4", cy: "4", r: "2", key: "top-left" }],
]);
