import type { AppStatus } from '../tauri-commands';

interface StatusCardProps {
  status: AppStatus;
  isChecking: boolean;
  onStatusCheck: () => void;
}

export const StatusCard = ({ status, isChecking, onStatusCheck }: StatusCardProps) => (
  <div className="status-card">
    <div>
      <span className="status-label">Status</span>
      <strong>{status.state}</strong>
      <p>{status.detail}</p>
    </div>
    <button type="button" onClick={onStatusCheck} disabled={isChecking}>
      {isChecking ? 'Checking...' : 'Check native status'}
    </button>
  </div>
);
