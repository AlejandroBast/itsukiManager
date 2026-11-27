import drive from "../assets/icons/drive.svg?raw";
import files from "../assets/icons/files.svg?raw";
import folder from "../assets/icons/folder.svg?raw";
import home from "../assets/icons/home.svg?raw";
import info from "../assets/icons/info.svg?raw";
import layers from "../assets/icons/layers.svg?raw";
import search from "../assets/icons/search.svg?raw";
import settings from "../assets/icons/settings.svg?raw";
import star from "../assets/icons/star.svg?raw";
import trash from "../assets/icons/trash.svg?raw";

/** Prepara un SVG del paquete de assets: el trazo principal toma el color del texto. */
function themed(svg: string): string {
  return svg
    .replace('stroke="#FBE8E6"', 'stroke="currentColor"')
    .replace('width="24" height="24"', 'width="100%" height="100%"');
}

/** Iconos sencillos que el paquete no trae, con el mismo estilo de trazo. */
const extra = {
  play: '<svg viewBox="0 0 24 24" width="100%" height="100%"><path d="M7 4.5v15a1 1 0 0 0 1.5.86l12-7.5a1 1 0 0 0 0-1.72l-12-7.5A1 1 0 0 0 7 4.5z" fill="currentColor"/></svg>',
  broom:
    '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><g transform="translate(2 -0.5) rotate(32 12 12)"><path d="M12 1.6v6.6"/><rect x="8.4" y="8.2" width="7.2" height="3.2" rx="1.2"/><path d="M8.9 11.4c-.6 3.1-2.1 6-3.9 8.2 2.2 1 4.6 1.5 7 1.5s4.8-.5 7-1.5c-1.8-2.2-3.3-5.1-3.9-8.2"/><path d="M10.4 15 9.3 20.6M13.6 15l1.1 5.6"/></g></svg>',
  minimize: '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M5 12h14"/></svg>',
  maximize:
    '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="5" y="5" width="14" height="14" rx="1.5"/></svg>',
  close: '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="m6 6 12 12M18 6 6 18"/></svg>',
  chevron: '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 5 7 7-7 7"/></svg>',
  arrow: '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12h15m-6-6 6 6-6 6"/></svg>',
  open: '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6M20 4l-8.5 8.5"/><path d="M19 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4"/></svg>',
  refresh:
    '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M20 11a8 8 0 0 0-14.3-4.9L4 8"/><path d="M4 4v4h4"/><path d="M4 13a8 8 0 0 0 14.3 4.9L20 16"/><path d="M20 20v-4h-4"/></svg>',
  check: '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12.5 4.5 4.5L19 7.5"/></svg>',
  warning:
    '<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.5 2.5 20h19z"/><path d="M12 10v4.5m0 2.5v.5"/></svg>',
};

const icons = {
  drive: themed(drive),
  files: themed(files),
  folder: themed(folder),
  home: themed(home),
  info: themed(info),
  layers: themed(layers),
  search: themed(search),
  settings: themed(settings),
  star: themed(star),
  trash: themed(trash),
  ...extra,
};

export type IconName = keyof typeof icons;

export function Icon({ name, className }: { name: IconName; className?: string }) {
  return <span className={className ? `icon ${className}` : "icon"} aria-hidden="true" dangerouslySetInnerHTML={{ __html: icons[name] }} />;
}
