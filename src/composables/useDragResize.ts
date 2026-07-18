import { ref } from "vue";

export function useDragResize(options: {
  direction: "horizontal" | "vertical";
  minSize: number;
  defaultSize: number;
  /** Set true when the panel is on the right/bottom side of the handle */
  reversed?: boolean;
  /** Initial size override (from persisted state) */
  initialSize?: number;
  /** Called when drag ends with the final size */
  onDragEnd?: (size: number) => void;
}) {
  const size = ref(options.initialSize ?? options.defaultSize);
  const isDragging = ref(false);

  function startDrag(e: MouseEvent) {
    e.preventDefault();
    isDragging.value = true;
    document.body.style.cursor =
      options.direction === "horizontal" ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";

    const startPos =
      options.direction === "horizontal" ? e.clientX : e.clientY;
    const startSize = size.value;

    function onMove(ev: MouseEvent) {
      const currentPos =
        options.direction === "horizontal" ? ev.clientX : ev.clientY;
      let delta =
        options.direction === "horizontal"
          ? currentPos - startPos
          : startPos - currentPos;
      if (options.reversed) delta = -delta;
      size.value = Math.max(options.minSize, startSize + delta);
    }

    function onUp() {
      isDragging.value = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      options.onDragEnd?.(size.value);
    }

    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }

  return { size, isDragging, startDrag };
}
