type ScryerLogoProps = {
  className?: string;
};

export default function ScryerLogo({ className = "" }: ScryerLogoProps) {
  return (
    <div className={`flex h-20 w-20 items-center ${className}`}>
      <img
        src="/logo.svg"
        alt="Scryer Logo"
        className="h-full w-full object-contain"
      />
    </div>
  );
}
