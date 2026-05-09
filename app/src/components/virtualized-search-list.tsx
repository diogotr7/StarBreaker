import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

export interface VirtualizedSearchListProps<T> {
  items: T[];
  rowHeight: number;
  estimateCount?: number;
  getKey: (item: T, index: number) => string;
  renderRow: (item: T, index: number) => React.ReactNode;
}

/**
 * Renders only the rows currently inside the scroll viewport.
 * Scales to millions of items because per-row work is bounded by viewport size.
 */
export function VirtualizedSearchList<T>({
  items,
  rowHeight,
  getKey,
  renderRow,
}: VirtualizedSearchListProps<T>) {
  const parentRef = useRef<HTMLDivElement>(null);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 8,
  });

  return (
    <div ref={parentRef} className="flex-1 min-h-0 overflow-y-auto">
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const item = items[virtualRow.index];
          return (
            <div
              key={getKey(item, virtualRow.index)}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: `${virtualRow.size}px`,
                transform: `translateY(${virtualRow.start}px)`,
              }}
            >
              {renderRow(item, virtualRow.index)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
