import { createContext } from "react";

/**
 * Lets content rendered into a gutter panel, including content portaled in
 * from elsewhere in the tree, drop its own card frame and offer the panel's
 * close button in its header.
 */
export const GutterPanelContext = createContext<{ close: () => void } | null>(null);
