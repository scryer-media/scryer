import * as React from "react";
import { useClient } from "urql";
import { instanceFeaturesQuery } from "@/lib/graphql/queries";
import { AUTH_SESSION_CHANGED_EVENT, getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { scheduleAfterFirstPaint } from "@/lib/utils/scheduling";
import type { InstanceFeatures } from "@/lib/types/settings";

/**
 * Until the query resolves, experimental surfaces stay hidden and personalized
 * discovery is assumed on. Those are the shipped defaults, so nothing flashes
 * open on a fresh install and no false "personalized discovery is off" hint
 * appears on an instance that has it on.
 */
export const DEFAULT_INSTANCE_FEATURES: InstanceFeatures = {
  experimentalFeaturesEnabled: false,
  personalizedDiscoveryEnabled: true,
};

type InstanceFeaturesContextValue = {
  instanceFeatures: InstanceFeatures;
  instanceFeaturesLoaded: boolean;
  refresh: () => Promise<void>;
};

const InstanceFeaturesContext =
  React.createContext<InstanceFeaturesContextValue | null>(null);

function normalizeInstanceFeatures(
  features: Partial<InstanceFeatures> | null | undefined,
): InstanceFeatures {
  return {
    experimentalFeaturesEnabled:
      features?.experimentalFeaturesEnabled ??
      DEFAULT_INSTANCE_FEATURES.experimentalFeaturesEnabled,
    personalizedDiscoveryEnabled:
      features?.personalizedDiscoveryEnabled ??
      DEFAULT_INSTANCE_FEATURES.personalizedDiscoveryEnabled,
  };
}

export function InstanceFeaturesProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const client = useClient();
  const [instanceFeatures, setInstanceFeatures] = React.useState<InstanceFeatures>(
    DEFAULT_INSTANCE_FEATURES,
  );
  const [instanceFeaturesLoaded, setInstanceFeaturesLoaded] = React.useState(false);
  const requestSequenceRef = React.useRef(0);

  const loadInstanceFeatures = React.useCallback(async () => {
    const requestId = requestSequenceRef.current + 1;
    requestSequenceRef.current = requestId;

    // The query needs a session. On the login surface with no token the
    // defaults are authoritative, and firing it there turns every auth-session
    // flap into a rejected-request storm.
    if (typeof window !== "undefined" && getAuthToken() === null) {
      const basePath = getRuntimeBasePath();
      const loginPath = basePath === "/" ? "/login" : `${basePath}/login`;
      if (window.location.pathname.startsWith(loginPath)) {
        setInstanceFeatures(DEFAULT_INSTANCE_FEATURES);
        setInstanceFeaturesLoaded(false);
        return;
      }
    }

    try {
      const { data, error } = await client
        .query<{ instanceFeatures?: Partial<InstanceFeatures> | null }>(
          instanceFeaturesQuery,
          {},
        )
        .toPromise();
      if (error) {
        throw error;
      }
      if (requestSequenceRef.current !== requestId) return;
      setInstanceFeatures(normalizeInstanceFeatures(data?.instanceFeatures));
      setInstanceFeaturesLoaded(true);
    } catch (error) {
      if (requestSequenceRef.current !== requestId) return;
      // Falling back to the defaults keeps unfinished surfaces hidden rather
      // than revealing them on a transient read failure.
      setInstanceFeatures(DEFAULT_INSTANCE_FEATURES);
      setInstanceFeaturesLoaded(false);
      console.warn("Failed to load instance features", error);
    }
  }, [client]);

  const refresh = React.useCallback(
    () => loadInstanceFeatures(),
    [loadInstanceFeatures],
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      void loadInstanceFeatures();
      return undefined;
    }

    const handleAuthSessionChanged = () => {
      void loadInstanceFeatures();
    };

    window.addEventListener(AUTH_SESSION_CHANGED_EVENT, handleAuthSessionChanged);
    const cancelScheduledQuery = scheduleAfterFirstPaint(() => {
      void loadInstanceFeatures();
    });
    return () => {
      requestSequenceRef.current += 1;
      cancelScheduledQuery();
      window.removeEventListener(
        AUTH_SESSION_CHANGED_EVENT,
        handleAuthSessionChanged,
      );
    };
  }, [loadInstanceFeatures]);

  const value = React.useMemo<InstanceFeaturesContextValue>(
    () => ({ instanceFeatures, instanceFeaturesLoaded, refresh }),
    [instanceFeatures, instanceFeaturesLoaded, refresh],
  );

  return (
    <InstanceFeaturesContext.Provider value={value}>
      {children}
    </InstanceFeaturesContext.Provider>
  );
}

/**
 * Read the instance-wide switches.
 *
 * Outside the provider this returns the shipped defaults rather than throwing,
 * so isolated component tests and any surface rendered before the provider
 * mounts still behave as an install with experimental features off.
 */
export function useInstanceFeatures(): InstanceFeatures {
  return (
    React.useContext(InstanceFeaturesContext)?.instanceFeatures ??
    DEFAULT_INSTANCE_FEATURES
  );
}

/** Refresh the switches, for example right after an administrator saves them. */
export function useRefreshInstanceFeatures(): () => Promise<void> {
  const value = React.useContext(InstanceFeaturesContext);
  return React.useMemo(
    () => value?.refresh ?? (() => Promise.resolve()),
    [value],
  );
}

/** Whether surfaces that are still being finished should render. */
export function useExperimentalFeaturesEnabled(): boolean {
  return useInstanceFeatures().experimentalFeaturesEnabled;
}
