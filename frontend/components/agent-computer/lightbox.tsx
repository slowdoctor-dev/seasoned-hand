"use client";

import { useEffect } from "react";

type Props = {
  src: string;
  alt: string;
  onClose: () => void;
};

export function Lightbox({ src, alt, onClose }: Props) {
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4"
      onClick={onClose}
      role="presentation"
    >
      <img
        src={src}
        alt={alt}
        className="max-h-[92vh] max-w-[92vw] rounded border border-white/20 bg-black object-contain"
        onClick={(e) => e.stopPropagation()}
      />
    </div>
  );
}
