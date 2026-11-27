import { useCallback, useEffect, useState } from "react";
import { api, isCancelled, type DiskInfo, type HomeSummary } from "./api";
import { AnalyzeView } from "./components/AnalyzeView";
import { CleanupBanner } from "./components/CleanupBanner";
import { CleanupView } from "./components/CleanupView";
import { DiskCard } from "./components/DiskCard";
import { DuplicatesView } from "./components/DuplicatesView";
import { FoldersCard } from "./components/FoldersCard";
import { LargeFilesView } from "./components/LargeFilesView";
import { SectionPlaceholder } from "./components/SectionPlaceholder";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { UsageCard } from "./components/UsageCard";
import { WelcomeBanner } from "./components/WelcomeBanner";

export function App() {
  const [section, setSection] = useState<Section>("inicio");
  // Las secciones visitadas siguen montadas (ocultas) para conservar sus resultados.
  const [visited, setVisited] = useState<Set<Section>>(() => new Set(["inicio"]));
  const [disk, setDisk] = useState<DiskInfo | null>(null);
  const [drives, setDrives] = useState<DiskInfo[]>([]);
  const [summary, setSummary] = useState<HomeSummary | null>(null);
  const [homeStale, setHomeStale] = useState(false);
  const [busy, setBusy] = useState(false);
  const [scannedFiles, setScannedFiles] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const go = useCallback((s: Section) => {
    setSection(s);
    setVisited((prev) => (prev.has(s) ? prev : new Set(prev).add(s)));
  }, []);

  const analyze = useCallback(async (refresh: boolean) => {
    setBusy(true);
    setError(null);
    const stop = await api.onScanProgress((p) => setScannedFiles(p.files)).catch(() => null);
    try {
      setSummary(await api.homeSummary(refresh));
      setDisk(await api.systemDisk());
    } catch (e) {
      if (!isCancelled(e)) setError(String(e));
    } finally {
      stop?.();
      setScannedFiles(null);
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    api.systemDisk().then(setDisk, (e) => setError(String(e)));
    api.listDrives().then(setDrives, () => setDrives([]));
    void analyze(false);
  }, [analyze]);

  // Tras borrar o limpiar desde otra sección, el inicio se recalcula al volver.
  const markChanged = useCallback(() => {
    setHomeStale(true);
    api.listDrives().then(setDrives, () => undefined);
  }, []);

  useEffect(() => {
    if (section === "inicio" && homeStale && !busy) {
      setHomeStale(false);
      void analyze(false);
    }
  }, [section, homeStale, busy, analyze]);

  const systemRoot = disk?.root ?? "C:\\";
  const view = (s: Section) => ({ className: "view", hidden: section !== s });

  return (
    <div className="app">
      <Sidebar section={section} onSelect={go} />
      <main className="main">
        <TitleBar />
        <div className="content">
          <div {...view("inicio")}>
            <WelcomeBanner />
            <DiskCard disk={disk} busy={busy} scannedFiles={scannedFiles} onAnalyze={() => void analyze(true)} />
            <div className="cards">
              <UsageCard summary={summary} />
              <FoldersCard folders={summary?.folders ?? null} onSeeAll={() => go("analizar")} />
            </div>
            <CleanupBanner reclaimable={summary?.reclaimable ?? null} onCleanup={() => go("limpieza")} />
          </div>
          {visited.has("analizar") && (
            <div {...view("analizar")}>
              <AnalyzeView drives={drives} systemRoot={systemRoot} onChanged={markChanged} />
            </div>
          )}
          {visited.has("grandes") && (
            <div {...view("grandes")}>
              <LargeFilesView drives={drives} systemRoot={systemRoot} onChanged={markChanged} />
            </div>
          )}
          {visited.has("duplicados") && (
            <div {...view("duplicados")}>
              <DuplicatesView drives={drives} onChanged={markChanged} />
            </div>
          )}
          {visited.has("limpieza") && (
            <div {...view("limpieza")}>
              <CleanupView onChanged={markChanged} />
            </div>
          )}
          {(section === "configuracion" || section === "acerca") && <SectionPlaceholder section={section} />}
        </div>
        {error && section === "inicio" && (
          <div className="toast" role="alert">
            No se pudo analizar el disco: {error}
            <button type="button" onClick={() => setError(null)}>
              Cerrar
            </button>
          </div>
        )}
      </main>
    </div>
  );
}
