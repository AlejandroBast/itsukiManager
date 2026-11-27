import { useCallback, useEffect, useState } from "react";
import { api, type DiskInfo, type HomeSummary } from "./api";
import { CleanupBanner } from "./components/CleanupBanner";
import { DiskCard } from "./components/DiskCard";
import { FoldersCard } from "./components/FoldersCard";
import { SectionPlaceholder } from "./components/SectionPlaceholder";
import { Sidebar, type Section } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { UsageCard } from "./components/UsageCard";
import { WelcomeBanner } from "./components/WelcomeBanner";

export function App() {
  const [section, setSection] = useState<Section>("inicio");
  const [disk, setDisk] = useState<DiskInfo | null>(null);
  const [summary, setSummary] = useState<HomeSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [scannedFiles, setScannedFiles] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const analyze = useCallback(async (refresh: boolean) => {
    setBusy(true);
    setError(null);
    const stop = await api.onScanProgress((p) => setScannedFiles(p.files)).catch(() => null);
    try {
      setSummary(await api.homeSummary(refresh));
      setDisk(await api.systemDisk());
    } catch (e) {
      setError(String(e));
    } finally {
      stop?.();
      setScannedFiles(null);
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    api.systemDisk().then(setDisk, (e) => setError(String(e)));
    void analyze(false);
  }, [analyze]);

  return (
    <div className="app">
      <Sidebar section={section} onSelect={setSection} />
      <main className="main">
        <TitleBar />
        <div className="content">
          {section === "inicio" ? (
            <>
              <WelcomeBanner />
              <DiskCard disk={disk} busy={busy} scannedFiles={scannedFiles} onAnalyze={() => void analyze(true)} />
              <div className="cards">
                <UsageCard summary={summary} />
                <FoldersCard folders={summary?.folders ?? null} onSeeAll={() => setSection("analizar")} />
              </div>
              <CleanupBanner reclaimable={summary?.reclaimable ?? null} onCleanup={() => setSection("limpieza")} />
            </>
          ) : (
            <SectionPlaceholder section={section} />
          )}
        </div>
        {error && (
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
