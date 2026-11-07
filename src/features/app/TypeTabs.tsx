import { TYPE_FILTERS, toggleTypeFilter, type TypeFilter } from "../../shared/lib/clipboardTypes";
import type { Translate } from "../../shared/types";

interface TypeTabsProps {
  t: Translate;
  value: TypeFilter | null;
  onChange: (filter: TypeFilter | null) => void;
}

const TypeTabs = ({ t, value, onChange }: TypeTabsProps) => (
  <div className="type-tabs" role="tablist">
    {TYPE_FILTERS.map((type) => (
      <button
        key={type}
        type="button"
        role="tab"
        aria-selected={value === type}
        className={`type-tab${value === type ? " is-active" : ""}`}
        onClick={() => onChange(toggleTypeFilter(value, type))}
      >
        {t(`type_${type}`)}
      </button>
    ))}
  </div>
);

export default TypeTabs;
