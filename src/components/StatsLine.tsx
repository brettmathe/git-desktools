import type { FileStats } from "../util";
import { PencilIcon, PlusIcon, RenameIcon, XIcon } from "./icons";

export function StatsLine({ stats }: { stats: FileStats }) {
  return (
    <div className="stats-line">
      {stats.modified > 0 && (
        <span className="st-modified">
          <PencilIcon size={12} /> {stats.modified} modified
        </span>
      )}
      {stats.added > 0 && (
        <span className="st-added">
          <PlusIcon size={12} /> {stats.added} added
        </span>
      )}
      {stats.deleted > 0 && (
        <span className="st-deleted">
          <XIcon size={12} /> {stats.deleted} deleted
        </span>
      )}
      {stats.renamed > 0 && (
        <span className="st-renamed">
          <RenameIcon size={12} /> {stats.renamed} renamed
        </span>
      )}
    </div>
  );
}
