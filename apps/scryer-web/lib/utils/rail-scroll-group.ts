export type ScrollableRail = { scrollLeft: number };

export type RailScrollGroup = {
  /** Joins a rail at the group's position; the returned function removes it. */
  add: (rail: ScrollableRail) => () => void;
  /** Call from a rail's scroll event to bring the other rails along. */
  scrolled: (rail: ScrollableRail) => void;
};

/**
 * Keeps several horizontal rails at one scroll position. A rail moved by the
 * group fires its own scroll event, and that echo must not be treated as the
 * user scrolling: a shorter rail clamps the position, so answering its echo
 * would drag the rail the user is scrolling back to the shorter rail's end.
 */
export function createRailScrollGroup(): RailScrollGroup {
  const rails = new Set<ScrollableRail>();
  const echoes = new WeakMap<ScrollableRail, number>();

  function moveTo(rail: ScrollableRail, left: number) {
    const before = rail.scrollLeft;
    rail.scrollLeft = left;
    if (rail.scrollLeft !== before) {
      echoes.set(rail, rail.scrollLeft);
    }
  }

  return {
    add(rail) {
      const [peer] = rails;
      rails.add(rail);
      if (peer) {
        moveTo(rail, peer.scrollLeft);
      }
      return () => {
        rails.delete(rail);
      };
    },
    scrolled(rail) {
      const echo = echoes.get(rail);
      echoes.delete(rail);
      if (echo === rail.scrollLeft) {
        return;
      }
      for (const peer of rails) {
        if (peer !== rail) {
          moveTo(peer, rail.scrollLeft);
        }
      }
    },
  };
}
