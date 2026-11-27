import mascot from "../assets/images/mascot-cleanup.png";
import { formatSize } from "../format";
import { useCountUp } from "../hooks";
import { Icon } from "./Icon";

interface Props {
  reclaimable: number | null;
  onCleanup: () => void;
}

/** Banner inferior: espacio que se puede liberar y acceso a la limpieza. */
export function CleanupBanner({ reclaimable, onCleanup }: Props) {
  const shown = useCountUp(reclaimable);
  return (
    <section className="cleanup rise" style={{ animationDelay: "240ms" }} aria-label="Limpieza">
      <img className="mascot" src={mascot} alt="" />
      <div className="cleanup-text">
        <div className="cleanup-title">
          Puedes liberar hasta
          {reclaimable === null ? (
            <span className="skeleton" style={{ width: "6.5rem", height: "1.9rem", alignSelf: "center" }} />
          ) : (
            <span className="amount">{formatSize(shown)}</span>
          )}
        </div>
        <div className="cleanup-sub">Elimina archivos temporales, duplicados y más.</div>
      </div>
      <button type="button" className="btn btn-outline" onClick={onCleanup}>
        <Icon name="broom" />
        Limpieza rápida
      </button>
    </section>
  );
}
