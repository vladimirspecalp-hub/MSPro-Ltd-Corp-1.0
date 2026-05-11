// Минимальный inline-SVG sparkline для последних 5-7 точек статистики.
// 0 зависимостей. Auto-scale на min/max, защита от плоских серий.

interface Props {
  values: number[];
  width?: number;
  height?: number;
  color?: string;
}

export default function Sparkline({
  values,
  width = 80,
  height = 22,
  color = "#666",
}: Props) {
  if (values.length < 2) {
    return (
      <svg width={width} height={height} aria-label="нет данных">
        <line
          x1={0}
          y1={height / 2}
          x2={width}
          y2={height / 2}
          stroke="#ddd"
          strokeDasharray="3 3"
          strokeWidth={1}
        />
      </svg>
    );
  }
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const pad = 2;
  const w = width - pad * 2;
  const h = height - pad * 2;
  const points = values
    .map((v, i) => {
      const x = pad + (i / (values.length - 1)) * w;
      const y = pad + h - ((v - min) / range) * h;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
  const lastX = pad + w;
  const lastY = pad + h - ((values[values.length - 1] - min) / range) * h;

  return (
    <svg width={width} height={height} aria-label="sparkline">
      <polyline
        fill="none"
        stroke={color}
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
        points={points}
      />
      <circle cx={lastX} cy={lastY} r={2.2} fill={color} />
    </svg>
  );
}
