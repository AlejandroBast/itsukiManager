import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon } from "./Icon";

/** Barra superior: zona para arrastrar la ventana y sus controles. La ventana tiene tamaño fijo (1080 × 720),
 * así que Maximizar se muestra desactivado, como en las ventanas fijas de Windows. */
export function TitleBar() {
  const win = () => getCurrentWindow();
  return (
    <header className="titlebar" data-tauri-drag-region>
      <button type="button" className="win-btn" aria-label="Minimizar" onClick={() => win().minimize()}>
        <Icon name="minimize" />
      </button>
      <button type="button" className="win-btn" aria-label="Maximizar" disabled>
        <Icon name="maximize" />
      </button>
      <button type="button" className="win-btn close" aria-label="Cerrar" onClick={() => win().close()}>
        <Icon name="close" />
      </button>
    </header>
  );
}
