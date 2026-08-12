use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

pub const DEFAULT_SUBNET_BITS: u8 = 24;
pub const DEFAULT_NETWORK: IpPrefix = IpPrefix {
    address: IpAddr::V4(Ipv4Addr::new(10, 210, 0, 0)),
    bits: 16,
};

/// An IP prefix that retains the input address while applying prefix semantics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IpPrefix {
    address: IpAddr,
    bits: u8,
}

impl IpPrefix {
    pub fn new(address: IpAddr, bits: u8) -> Result<Self, IpPrefixError> {
        if bits > bit_len(address) {
            return Err(IpPrefixError::InvalidPrefix);
        }
        Ok(Self { address, bits })
    }

    #[must_use]
    pub const fn address(self) -> IpAddr {
        self.address
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.bits
    }

    #[must_use]
    pub fn masked(self) -> Self {
        Self {
            address: from_u128(self.address, network_value(self)),
            bits: self.bits,
        }
    }

    #[must_use]
    pub fn contains(self, address: IpAddr) -> bool {
        same_family(self.address, address)
            && (to_u128(address) & prefix_mask(self.address, self.bits)) == network_value(self)
    }

    fn last_address(self) -> IpAddr {
        let host_mask = !prefix_mask(self.address, self.bits) & address_mask(self.address);
        from_u128(self.address, network_value(self) | host_mask)
    }

    fn overlaps(self, other: Self) -> bool {
        same_family(self.address, other.address) && self.contains(other.masked().address)
            || same_family(self.address, other.address) && other.contains(self.masked().address)
    }
}

impl fmt::Display for IpPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.address, self.bits)
    }
}

impl FromStr for IpPrefix {
    type Err = IpPrefixError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (address, bits) = value.rsplit_once('/').ok_or(IpPrefixError::InvalidPrefix)?;
        if bits.is_empty()
            || !bits.bytes().all(|byte| byte.is_ascii_digit())
            || (bits.len() > 1 && bits.starts_with('0'))
        {
            return Err(IpPrefixError::InvalidPrefix);
        }
        let address = address
            .parse::<IpAddr>()
            .map_err(|_| IpPrefixError::InvalidPrefix)?;
        let bits = bits
            .parse::<u8>()
            .map_err(|_| IpPrefixError::InvalidPrefix)?;
        Self::new(address, bits)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IpPrefixError {
    InvalidNetwork,
    InvalidSubnetSize,
    SubnetNotInNetwork,
    SubnetOverlap,
    NoAvailableSubnet,
    InvalidPrefix,
    AllocatedSubnet {
        subnet: IpPrefix,
        source: Box<IpPrefixError>,
    },
}

impl fmt::Display for IpPrefixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidNetwork => "invalid network",
            Self::InvalidSubnetSize => "invalid subnet size",
            Self::SubnetNotInNetwork => "subnet not in network",
            Self::SubnetOverlap => "subnet overlaps with allocated subnets",
            Self::NoAvailableSubnet => "no available subnet",
            Self::InvalidPrefix => "invalid prefix",
            Self::AllocatedSubnet { subnet, source } => {
                return write!(formatter, "allocate subnet {subnet}: {source}");
            }
        })
    }
}

impl std::error::Error for IpPrefixError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AllocatedSubnet { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// In-memory first-fit allocator for machine subnets in one cluster network.
#[derive(Clone, Debug)]
pub struct Ipam {
    network: IpPrefix,
    allocated: Vec<IpPrefix>,
}

impl Ipam {
    pub fn new(network: IpPrefix) -> Result<Self, IpPrefixError> {
        if network.bits == 0 {
            return Err(IpPrefixError::InvalidNetwork);
        }
        Ok(Self {
            network: network.masked(),
            allocated: Vec::new(),
        })
    }

    pub fn with_allocated(
        network: IpPrefix,
        subnets: impl IntoIterator<Item = IpPrefix>,
    ) -> Result<Self, IpPrefixError> {
        let mut allocator = Self::new(network)?;
        for subnet in subnets {
            allocator
                .allocate_subnet(subnet)
                .map_err(|error| error.with_allocation_context(subnet))?;
        }
        Ok(allocator)
    }

    pub fn allocate_subnet_len(&mut self, bits: u8) -> Result<IpPrefix, IpPrefixError> {
        if bits < self.network.bits || bits > bit_len(self.network.address) {
            return Err(IpPrefixError::InvalidSubnetSize);
        }

        let mut address = self.network.address;
        while self.network.contains(address) {
            let subnet = IpPrefix { address, bits };
            if !self.allocated.iter().any(|used| used.overlaps(subnet)) {
                self.allocated.push(subnet);
                return Ok(subnet);
            }
            let last = to_u128(subnet.last_address());
            let Some(next) = last
                .checked_add(1)
                .filter(|next| *next <= address_mask(address))
            else {
                break;
            };
            address = from_u128(address, next);
        }
        Err(IpPrefixError::NoAvailableSubnet)
    }

    pub fn allocate_subnet(&mut self, subnet: IpPrefix) -> Result<(), IpPrefixError> {
        if !self.network.contains(subnet.address) || !self.network.contains(subnet.last_address()) {
            return Err(IpPrefixError::SubnetNotInNetwork);
        }
        if self.allocated.iter().any(|used| used.overlaps(subnet)) {
            return Err(IpPrefixError::SubnetOverlap);
        }
        self.allocated.push(subnet);
        Ok(())
    }
}

impl IpPrefixError {
    fn with_allocation_context(self, subnet: IpPrefix) -> Self {
        Self::AllocatedSubnet {
            subnet,
            source: Box::new(self),
        }
    }
}

const fn bit_len(address: IpAddr) -> u8 {
    match address {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}

const fn same_family(left: IpAddr, right: IpAddr) -> bool {
    matches!(
        (left, right),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

fn address_mask(address: IpAddr) -> u128 {
    match address {
        IpAddr::V4(_) => u128::from(u32::MAX),
        IpAddr::V6(_) => u128::MAX,
    }
}

fn prefix_mask(address: IpAddr, bits: u8) -> u128 {
    match address {
        IpAddr::V4(_) => {
            if bits == 0 {
                0
            } else {
                u128::from(u32::MAX << (32 - bits))
            }
        }
        IpAddr::V6(_) => {
            if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            }
        }
    }
}

fn network_value(prefix: IpPrefix) -> u128 {
    to_u128(prefix.address) & prefix_mask(prefix.address, prefix.bits)
}

fn to_u128(address: IpAddr) -> u128 {
    match address {
        IpAddr::V4(address) => u128::from(u32::from(address)),
        IpAddr::V6(address) => u128::from(address),
    }
}

fn from_u128(family: IpAddr, value: u128) -> IpAddr {
    match family {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::from(
            u32::try_from(value).expect("IPv4 arithmetic stays within 32 bits"),
        )),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::from(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefix(value: &str) -> IpPrefix {
        value.parse().expect("valid fixture")
    }

    #[test]
    fn rejects_zero_length_network() {
        assert_eq!(
            Ipam::new(prefix("0.0.0.0/0")).unwrap_err(),
            IpPrefixError::InvalidNetwork
        );
    }

    #[test]
    fn masks_network_and_allocates_first_free_subnets() {
        let mut ipam = Ipam::new(prefix("10.210.7.9/16")).expect("valid network");
        assert_eq!(
            ipam.allocate_subnet_len(24).unwrap(),
            prefix("10.210.0.0/24")
        );
        assert_eq!(
            ipam.allocate_subnet_len(24).unwrap(),
            prefix("10.210.1.0/24")
        );
    }

    #[test]
    fn preallocated_ranges_are_validated_and_skipped() {
        let mut ipam = Ipam::with_allocated(
            prefix("10.210.0.0/16"),
            [prefix("10.210.0.0/23"), prefix("10.210.2.0/24")],
        )
        .expect("valid allocations");
        assert_eq!(
            ipam.allocate_subnet_len(24).unwrap(),
            prefix("10.210.3.0/24")
        );
        assert_eq!(
            ipam.allocate_subnet(prefix("10.210.2.128/25")),
            Err(IpPrefixError::SubnetOverlap)
        );
    }

    #[test]
    fn preallocated_error_retains_subnet_context() {
        let error =
            Ipam::with_allocated(prefix("10.210.0.0/16"), [prefix("10.211.0.0/24")]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "allocate subnet 10.211.0.0/24: subnet not in network"
        );
    }

    #[test]
    fn rejects_out_of_network_and_invalid_sizes() {
        let mut ipam = Ipam::new(prefix("10.210.0.0/16")).unwrap();
        assert_eq!(
            ipam.allocate_subnet(prefix("10.211.0.0/24")),
            Err(IpPrefixError::SubnetNotInNetwork)
        );
        assert_eq!(
            ipam.allocate_subnet_len(15),
            Err(IpPrefixError::InvalidSubnetSize)
        );
        assert_eq!(
            ipam.allocate_subnet_len(33),
            Err(IpPrefixError::InvalidSubnetSize)
        );
    }

    #[test]
    fn parser_rejects_noncanonical_go_prefix_lengths() {
        for value in ["10.210.0.0/016", "10.210.0.0/+16", "10.210.0.0/-1"] {
            assert_eq!(value.parse::<IpPrefix>(), Err(IpPrefixError::InvalidPrefix));
        }
    }

    #[test]
    fn reports_exhaustion_for_ipv4_and_ipv6() {
        let mut v4 = Ipam::new(prefix("192.0.2.4/32")).unwrap();
        assert_eq!(v4.allocate_subnet_len(32).unwrap(), prefix("192.0.2.4/32"));
        assert_eq!(
            v4.allocate_subnet_len(32),
            Err(IpPrefixError::NoAvailableSubnet)
        );

        let mut maximum_v4 = Ipam::new(prefix("255.255.255.255/32")).unwrap();
        assert_eq!(
            maximum_v4.allocate_subnet_len(32).unwrap(),
            prefix("255.255.255.255/32")
        );
        assert_eq!(
            maximum_v4.allocate_subnet_len(32),
            Err(IpPrefixError::NoAvailableSubnet)
        );

        let mut v6 = Ipam::new(prefix("2001:db8::/127")).unwrap();
        assert_eq!(
            v6.allocate_subnet_len(128).unwrap(),
            prefix("2001:db8::/128")
        );
        assert_eq!(
            v6.allocate_subnet_len(128).unwrap(),
            prefix("2001:db8::1/128")
        );
        assert_eq!(
            v6.allocate_subnet_len(128),
            Err(IpPrefixError::NoAvailableSubnet)
        );
    }
}
