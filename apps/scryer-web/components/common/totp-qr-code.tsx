import qrcode from "qrcode-generator";
import { useMemo } from "react";
import { cn } from "@/lib/utils";

const TOTP_QR_CELL_SIZE = 8;
const TOTP_QR_QUIET_ZONE_MODULES = 4;

type TotpQrCodeProps = {
  value: string;
  id?: string;
  className?: string;
  ariaLabel?: string;
};

export function TotpQrCode({
  value,
  id,
  className,
  ariaLabel = "TOTP setup QR code",
}: TotpQrCodeProps) {
  const image = useMemo(() => {
    const code = qrcode(0, "L");
    code.addData(value);
    code.make();
    const pixelSize =
      (code.getModuleCount() + TOTP_QR_QUIET_ZONE_MODULES * 2) *
      TOTP_QR_CELL_SIZE;

    return {
      pixelSize,
      src: code.createDataURL(TOTP_QR_CELL_SIZE, TOTP_QR_QUIET_ZONE_MODULES),
    };
  }, [value]);

  return (
    <div
      id={id}
      className={cn(
        "inline-flex max-w-full rounded-md bg-white shadow-sm",
        className,
      )}
    >
      <img
        alt={ariaLabel}
        className="block h-auto max-w-full [image-rendering:pixelated]"
        height={image.pixelSize}
        src={image.src}
        width={image.pixelSize}
      />
    </div>
  );
}
