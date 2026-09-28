import { splitProps, type Component, type JSX, type ParentProps } from "solid-js";
import { handleWindowDrag } from "@/lib/window-drag";

type SafeDragSurfaceProps = ParentProps<JSX.HTMLAttributes<HTMLDivElement>> & {
  directTargetOnly?: boolean;
};

const SafeDragSurface: Component<SafeDragSurfaceProps> = (props) => {
  const [local, rest] = splitProps(props, ["directTargetOnly"]);
  const onMouseDown = (event: MouseEvent) => {
    if (local.directTargetOnly && event.target !== event.currentTarget) return;
    handleWindowDrag(event);
  };

  return <div {...rest} onMouseDown={onMouseDown} />;
};

export default SafeDragSurface;
