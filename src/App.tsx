import { useState } from "react";
import Sidebar, { type View } from "./components/Sidebar";
import Home from "./components/views/Home";
import CeoChat from "./components/views/CeoChat";
import SecurityVault from "./components/views/SecurityVault";
import Dispatcher from "./components/views/Dispatcher";
import Settings from "./components/views/Settings";
import { ToastProvider } from "./components/common/Toast";
import "./App.css";

export default function App() {
  const [view, setView] = useState<View>("home");

  return (
    <ToastProvider>
      <div
        style={{
          display: "flex",
          height: "100vh",
          overflow: "hidden",
          fontFamily: "system-ui, -apple-system, sans-serif",
          color: "#1a1a1a",
          background: "#f9f9f9",
        }}
      >
        <Sidebar current={view} onChange={setView} />
        <main style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          {view === "home" && <Home />}
          {view === "ceo" && <CeoChat />}
          {view === "vault" && <SecurityVault />}
          {view === "dispatcher" && <Dispatcher />}
          {view === "settings" && <Settings />}
        </main>
      </div>
    </ToastProvider>
  );
}
