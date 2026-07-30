import { UUID } from './utils';
import * as NetworkTypes from '../types/network';
import { NetworkConfig, NetworkInstanceRunningInfo, VpnPortalInfo } from '../types/network';

export interface ValidateConfigResponse {
    toml_config: string;
}

export interface ListNetworkInstanceIdResponse {
    running_inst_ids: Array<UUID>,
    // Enabled desired-state configs that are NOT currently running. When the
    // device is offline these are the configs that will be pushed on the next
    // reconnect ("pending on reconnect").
    enabled_inst_ids: Array<UUID>,
    disabled_inst_ids: Array<UUID>,
    // Device-owned (source != 'web') desired-state configs. The console can take
    // these over while the device is offline; the UI surfaces them with a
    // "pending takeover" affordance.
    user_inst_ids: Array<UUID>,
}

export interface GenerateConfigResponse {
    toml_config?: string;
    error?: string;
}

export interface ParseConfigResponse {
    config?: NetworkConfig;
    error?: string;
}

export interface CollectNetworkInfoResponse {
    info: {
        map: Record<string, NetworkInstanceRunningInfo | undefined>;
    }
}

export namespace ConfigFilePermission {
    export type Flags = number;
    export const READ_ONLY: Flags = 1 << 0;
    export const NO_DELETE: Flags = 1 << 1;
    export function hasPermission(perm: Flags, flag: Flags): boolean {
        return (perm & flag) === flag;
    }
    export function isRemoveSaveable(perm: Flags): boolean {
        return !hasPermission(perm, NO_DELETE);
    }
    export function isEditable(perm: Flags): boolean {
        return !hasPermission(perm, READ_ONLY);
    }
    export function isDeletable(perm: Flags): boolean {
        return !hasPermission(perm, NO_DELETE);
    }
}

export interface NetworkMeta {
    network_name: string;
    config_permission: ConfigFilePermission.Flags;
}

export interface GetNetworkMetasResponse {
    metas: Record<string, NetworkMeta>;
}

export interface RemoteClient {
    validate_config(config: NetworkConfig): Promise<ValidateConfigResponse>;
    run_network(config: NetworkConfig, save: boolean): Promise<undefined>;
    get_network_info(inst_id: string): Promise<NetworkInstanceRunningInfo | undefined>;
    get_vpn_portal_info(inst_id: string): Promise<VpnPortalInfo | undefined>;
    add_vpn_portal_client(inst_id: string, client: { name: string, virtual_ip: string, groups: string[] }): Promise<undefined>;
    remove_vpn_portal_client(inst_id: string, name: string): Promise<undefined>;
    clear_vpn_portal_clients(inst_id: string): Promise<undefined>;
    list_network_instance_ids(): Promise<ListNetworkInstanceIdResponse>;
    delete_network(inst_id: string): Promise<undefined>;
    update_network_instance_state(inst_id: string, disabled: boolean): Promise<undefined>;
    save_config(config: NetworkConfig): Promise<undefined>;
    get_network_config(inst_id: string): Promise<NetworkConfig>;
    generate_config(config: NetworkConfig): Promise<GenerateConfigResponse>;
    parse_config(toml_config: string): Promise<ParseConfigResponse>;
    get_network_metas(instance_ids: string[]): Promise<GetNetworkMetasResponse>;
}
// Global (user-scoped) preset-network-group API. Declared as an interface so the
// lib components can depend on it without importing the host's ApiClient. The host
// ApiClient implements these methods structurally.
export interface PresetClient {
    list_presets(): Promise<Array<NetworkTypes.PresetSummary>>;
    create_preset(name: string, config: NetworkTypes.NetworkConfig): Promise<NetworkTypes.PresetSummary>;
    update_preset(
        preset_id: number,
        name: string,
        config: NetworkTypes.NetworkConfig,
    ): Promise<NetworkTypes.PresetSummary>;
    delete_preset(preset_id: number): Promise<undefined>;
    join_preset(preset_id: number, machine_id: string): Promise<string>;
    get_preset_networks(preset_id: number): Promise<Array<NetworkTypes.PresetNetwork>>;
    // Config-file serialization (edit-as-file). Backed by the global
    // generate_config / parse_config endpoints on the host ApiClient.
    generate_config(config: NetworkTypes.NetworkConfig): Promise<GenerateConfigResponse>;
    parse_config(toml_config: string): Promise<ParseConfigResponse>;
}
