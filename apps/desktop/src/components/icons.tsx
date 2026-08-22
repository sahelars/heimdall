/**
 * The toolbar and header icons.
 *
 * Inline SVG rather than a font or a package: twelve small shapes, all drawn in
 * `currentColor` so they inherit the theme without a single colour literal
 * living in TypeScript.
 */

interface IconProps {
  size?: number;
}

function Svg({ size = 16, children }: IconProps & { children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.2"
      strokeLinecap="square"
      strokeLinejoin="miter"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

export function IconNewNote(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 2h6l4 4v8H3z" />
      <path d="M9 2v4h4" />
      <path d="M6 9h4M8 7v4" />
    </Svg>
  );
}

export function IconNewFolder(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M2 4h4l1 2h7v7H2z" />
      <path d="M6 9.5h4M8 7.5v4" />
    </Svg>
  );
}

export function IconSort(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 12V3M2 5l2-2 2 2" />
      <path d="M8 5h6M8 8h5M8 11h4" />
    </Svg>
  );
}

export function IconCollapse(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M2 3h12v10H2z" />
      <path d="M6 3v10" />
    </Svg>
  );
}

export function IconClose(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 4l8 8M12 4l-8 8" />
    </Svg>
  );
}

export function IconChevronRight(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M6 3l5 5-5 5" />
    </Svg>
  );
}

export function IconChevronDown(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 6l5 5 5-5" />
    </Svg>
  );
}

export function IconBack(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M13 8H3M6 4L2.5 8 6 12" />
    </Svg>
  );
}

export function IconForward(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 8h10M10 4l3.5 4L10 12" />
    </Svg>
  );
}

/** Shown in preview mode: click to edit the source. */
export function IconEdit(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M11 2.5l2.5 2.5L6 12.5 3 13l.5-3z" />
    </Svg>
  );
}

/** Shown in source mode: click to read the rendered note. */
export function IconRead(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M8 4.5S6.5 3 4 3H2v9h2c2.5 0 4 1.5 4 1.5S9.5 12 12 12h2V3h-2c-2.5 0-4 1.5-4 1.5z" />
      <path d="M8 4.5v9" />
    </Svg>
  );
}

export function IconMore(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3.5 8h.01M8 8h.01M12.5 8h.01" strokeWidth="1.8" />
    </Svg>
  );
}
