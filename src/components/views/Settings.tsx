import UpdateRollback from "../settings/UpdateRollback";
import ExternalAgentGateway from "../settings/ExternalAgentGateway";
import VaultPreview from "../settings/VaultPreview";

export default function Settings() {
  return (
    <div style={{ padding: "32px 48px", maxWidth: 900 }}>
      <header style={{ borderBottom: "2px solid #1a1a1a", paddingBottom: 16, marginBottom: 24 }}>
        <h1 style={{ margin: 0, fontSize: 28 }}>Настройки</h1>
        <p style={{ margin: "4px 0 0", color: "#666", fontSize: 14 }}>
          Обновления, бэкапы, режим разработчика, память Гендира
        </p>
      </header>
      <UpdateRollback />
      <ExternalAgentGateway />
      <VaultPreview />
    </div>
  );
}
