// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use {
    crate::{
        error::{DebianError, Result},
        io::{ContentDigest, DataResolver, MultiDigester, Compression},
        repository::{
            release::ReleaseFile, ReleaseReader, RepositoryPathVerification, RepositoryPathVerificationState, RepositoryRootReader,
            RepositoryWrite, RepositoryWriter,
        },
    },
    async_trait::async_trait,
    aws_config::Region,
    aws_sdk_s3::{
        error::SdkError,
        primitives::ByteStream,
        Client,
    },
    futures::{AsyncRead, AsyncReadExt as FuturesAsyncReadExt},
    std::{borrow::Cow, pin::Pin},
    tokio::io::AsyncReadExt as TokioAsyncReadExt,
    tokio_util::compat::TokioAsyncReadCompatExt,
    url::Url,
};

/// S3-based repository client for reading Debian repositories.
///
/// This client can read repository data from S3 buckets and implements both
/// [RepositoryRootReader] for reading repository metadata and [DataResolver]
/// for reading arbitrary paths.
#[derive(Clone)]
pub struct S3RepositoryClient {
    client: Client,
    bucket: String,
    key_prefix: Option<String>,
    region: Region,
}

impl S3RepositoryClient {
    /// Create a new S3 repository client bound to a named bucket with optional key prefix.
    ///
    /// This will use the default AWS configuration from the environment.
    pub async fn new(bucket: impl ToString, key_prefix: Option<&str>) -> Result<Self> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = Client::new(&config);
        let region = config.region().cloned().unwrap_or_else(|| Region::new("us-east-1"));

        Ok(Self {
            client,
            bucket: bucket.to_string(),
            key_prefix: key_prefix.map(|x| x.trim_matches('/').to_string()),
            region,
        })
    }

    /// Create a new S3 repository client with explicit region.
    pub async fn new_with_region(region: Region, bucket: impl ToString, key_prefix: Option<&str>) -> Result<Self> {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest()).region(region.clone()).load().await;
        let client = Client::new(&config);

        Ok(Self {
            client,
            bucket: bucket.to_string(),
            key_prefix: key_prefix.map(|x| x.trim_matches('/').to_string()),
            region,
        })
    }

    /// Create a new S3 repository client with a pre-configured AWS client.
    pub fn new_with_client(client: Client, region: Region, bucket: impl ToString, key_prefix: Option<&str>) -> Self {
        Self {
            client,
            bucket: bucket.to_string(),
            key_prefix: key_prefix.map(|x| x.trim_matches('/').to_string()),
            region,
        }
    }

    /// Compute the S3 key name given a repository relative path.
    pub fn path_to_key(&self, path: &str) -> String {
        if let Some(prefix) = &self.key_prefix {
            format!("{}/{}", prefix, path.trim_matches('/'))
        } else {
            path.trim_matches('/').to_string()
        }
    }

    /// Get the bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Get the key prefix (if any).
    pub fn key_prefix(&self) -> Option<&str> {
        self.key_prefix.as_deref()
    }

    /// Get the AWS region.
    pub fn region(&self) -> &Region {
        &self.region
    }
}

#[async_trait]
impl DataResolver for S3RepositoryClient {
    async fn get_path(&self, path: &str) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
        let key = self.path_to_key(path);

        match self.client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(output) => {
                let body = output.body.into_async_read().compat();
                Ok(Box::pin(body))
            }
            Err(SdkError::ServiceError(service_err)) => {
                if service_err.err().is_no_such_key() {
                    Err(DebianError::RepositoryIoPath(
                        path.to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("S3 key not found: {}", key),
                        ),
                    ))
                } else {
                    Err(DebianError::RepositoryIoPath(
                        path.to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("S3 error: {:?}", service_err),
                        ),
                    ))
                }
            }
            Err(e) => Err(DebianError::RepositoryIoPath(
                path.to_string(),
                std::io::Error::new(std::io::ErrorKind::Other, format!("S3 error: {:?}", e)),
            )),
        }
    }
}

#[async_trait]
impl RepositoryRootReader for S3RepositoryClient {
    fn url(&self) -> Result<Url> {
        let base = if let Some(prefix) = &self.key_prefix {
            format!("s3://{}/{}", self.bucket, prefix)
        } else {
            format!("s3://{}", self.bucket)
        };
        
        Url::parse(&base).map_err(|e| DebianError::Other(format!("Invalid S3 URL: {}", e)))
    }

    async fn release_reader_with_distribution_path(
        &self,
        path: &str,
    ) -> Result<Box<dyn ReleaseReader>> {
        let distribution_path = path.trim_matches('/').to_string();
        let inrelease_path = format!("{}/InRelease", distribution_path);
        let release_path = format!("{}/Release", distribution_path);

        let release = self
            .fetch_inrelease_or_release(&inrelease_path, &release_path)
            .await?;

        let fetch_compression = Compression::default_preferred_order()
            .next()
            .expect("iterator should not be empty");

        Ok(Box::new(S3ReleaseClient {
            client: self.client.clone(),
            bucket: self.bucket.clone(),
            key_prefix: self.key_prefix.clone(),
            relative_path: distribution_path,
            release,
            fetch_compression,
        }))
    }
}

/// S3-based release reader for a specific distribution.
///
/// This is created by [S3RepositoryClient] when reading a specific distribution
/// and provides access to packages, sources, and contents files.
pub struct S3ReleaseClient {
    client: Client,
    bucket: String,
    key_prefix: Option<String>,
    relative_path: String,
    release: ReleaseFile<'static>,
    fetch_compression: Compression,
}

impl S3ReleaseClient {
    /// Compute the S3 key name given a repository relative path.
    pub fn path_to_key(&self, path: &str) -> String {
        if let Some(prefix) = &self.key_prefix {
            format!("{}/{}", prefix, path.trim_matches('/'))
        } else {
            path.trim_matches('/').to_string()
        }
    }
}

#[async_trait]
impl DataResolver for S3ReleaseClient {
    async fn get_path(&self, path: &str) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
        let full_path = format!("{}/{}", self.relative_path, path.trim_matches('/'));
        let key = self.path_to_key(&full_path);

        match self.client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(output) => {
                let body = output.body.into_async_read().compat();
                Ok(Box::pin(body))
            }
            Err(SdkError::ServiceError(service_err)) => {
                if service_err.err().is_no_such_key() {
                    Err(DebianError::RepositoryIoPath(
                        path.to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("S3 key not found: {}", key),
                        ),
                    ))
                } else {
                    Err(DebianError::RepositoryIoPath(
                        path.to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("S3 error: {:?}", service_err),
                        ),
                    ))
                }
            }
            Err(e) => Err(DebianError::RepositoryIoPath(
                path.to_string(),
                std::io::Error::new(std::io::ErrorKind::Other, format!("S3 error: {:?}", e)),
            )),
        }
    }
}

#[async_trait]
impl ReleaseReader for S3ReleaseClient {
    fn url(&self) -> Result<Url> {
        let base = if let Some(prefix) = &self.key_prefix {
            format!("s3://{}/{}/{}", self.bucket, prefix, self.relative_path)
        } else {
            format!("s3://{}/{}", self.bucket, self.relative_path)
        };
        
        Url::parse(&base).map_err(|e| DebianError::Other(format!("Invalid S3 URL: {}", e)))
    }

    fn root_relative_path(&self) -> &str {
        &self.relative_path
    }

    fn release_file(&self) -> &ReleaseFile<'static> {
        &self.release
    }

    fn preferred_compression(&self) -> Compression {
        self.fetch_compression
    }

    fn set_preferred_compression(&mut self, compression: Compression) {
        self.fetch_compression = compression;
    }
}

pub struct S3Writer {
    client: Client,
    bucket: String,
    key_prefix: Option<String>,
}

impl S3Writer {
    /// Create a new S3 writer bound to a named bucket with optional key prefix.
    ///
    /// This will use the default AWS configuration from the environment.
    pub async fn new(bucket: impl ToString, key_prefix: Option<&str>) -> Result<Self> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = Client::new(&config);

        Ok(Self {
            client,
            bucket: bucket.to_string(),
            key_prefix: key_prefix.map(|x| x.trim_matches('/').to_string()),
        })
    }

    /// Create a new S3 writer with explicit region.
    pub async fn new_with_region(region: Region, bucket: impl ToString, key_prefix: Option<&str>) -> Result<Self> {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest()).region(region).load().await;
        let client = Client::new(&config);

        Ok(Self {
            client,
            bucket: bucket.to_string(),
            key_prefix: key_prefix.map(|x| x.trim_matches('/').to_string()),
        })
    }

    /// Create a new S3 writer with a pre-configured AWS client.
    pub fn new_with_client(client: Client, bucket: impl ToString, key_prefix: Option<&str>) -> Self {
        Self {
            client,
            bucket: bucket.to_string(),
            key_prefix: key_prefix.map(|x| x.trim_matches('/').to_string()),
        }
    }

    /// Compute the S3 key name given a repository relative path.
    pub fn path_to_key(&self, path: &str) -> String {
        if let Some(prefix) = &self.key_prefix {
            format!("{}/{}", prefix, path.trim_matches('/'))
        } else {
            path.trim_matches('/').to_string()
        }
    }
}

#[async_trait]
impl RepositoryWriter for S3Writer {
    async fn verify_path<'path>(
        &self,
        path: &'path str,
        expected_content: Option<(u64, ContentDigest)>,
    ) -> Result<RepositoryPathVerification<'path>> {
        let key = self.path_to_key(path);

        if let Some((expected_size, expected_digest)) = expected_content {
            match self.client
                .get_object()
                .bucket(&self.bucket)
                .key(&key)
                .send()
                .await
            {
                Ok(output) => {
                    // Fast path without having to read the body.
                    if let Some(cl) = output.content_length() {
                        if cl as u64 != expected_size {
                            return Ok(RepositoryPathVerification {
                                path,
                                state: RepositoryPathVerificationState::ExistsIntegrityMismatch,
                            });
                        }
                    }

                    let mut digester = MultiDigester::default();
                    let mut remaining = expected_size;
                    let mut reader = output.body.into_async_read();
                    let mut buf = [0u8; 16384];

                    loop {
                        let size = reader
                            .read(&mut buf[..])
                            .await
                            .map_err(|e| DebianError::RepositoryIoPath(path.to_string(), e))?;

                        digester.update(&buf[0..size]);

                        let size = size as u64;

                        if size >= remaining || size == 0 {
                            break;
                        }

                        remaining -= size;
                    }

                    let digests = digester.finish();

                    Ok(RepositoryPathVerification {
                        path,
                        state: if !digests.matches_digest(&expected_digest) {
                            RepositoryPathVerificationState::ExistsIntegrityMismatch
                        } else {
                            RepositoryPathVerificationState::ExistsIntegrityVerified
                        },
                    })
                }
                Err(SdkError::ServiceError(service_err)) if service_err.err().is_no_such_key() => {
                    Ok(RepositoryPathVerification {
                        path,
                        state: RepositoryPathVerificationState::Missing,
                    })
                }
                Err(e) => Err(DebianError::RepositoryIoPath(
                    path.to_string(),
                    std::io::Error::other(format!("S3 error: {:?}", e)),
                )),
            }
        } else {
            match self.client
                .head_object()
                .bucket(&self.bucket)
                .key(&key)
                .send()
                .await
            {
                Ok(_) => Ok(RepositoryPathVerification {
                    path,
                    state: RepositoryPathVerificationState::ExistsNoIntegrityCheck,
                }),
                Err(SdkError::ServiceError(service_err)) if service_err.err().is_not_found() => {
                    Ok(RepositoryPathVerification {
                        path,
                        state: RepositoryPathVerificationState::Missing,
                    })
                }
                Err(e) => Err(DebianError::RepositoryIoPath(
                    path.to_string(),
                    std::io::Error::other(format!("S3 error: {:?}", e)),
                )),
            }
        }
    }

    async fn write_path<'path, 'reader>(
        &self,
        path: Cow<'path, str>,
        mut reader: Pin<Box<dyn AsyncRead + Send + 'reader>>,
    ) -> Result<RepositoryWrite<'path>> {
        // Convert AsyncRead to ByteStream
        let mut buf = vec![];
        reader
            .read_to_end(&mut buf)
            .await
            .map_err(|e| DebianError::RepositoryIoPath(path.to_string(), e))?;

        let bytes_written = buf.len() as u64;
        let byte_stream = ByteStream::from(buf);
        let key = self.path_to_key(path.as_ref());

        match self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(byte_stream)
            .send()
            .await
        {
            Ok(_) => Ok(RepositoryWrite {
                path,
                bytes_written,
            }),
            Err(e) => Err(DebianError::RepositoryIoPath(
                path.to_string(),
                std::io::Error::other(format!("S3 error: {:?}", e)),
            )),
        }
    }
}

/// Attempt to resolve the AWS region of an S3 bucket.
pub async fn get_bucket_region(bucket: impl ToString) -> Result<Region> {
    // Use us-east-1 as default region for the initial API call to get_bucket_location
    // The bucket's actual region will be returned from the API call
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .load()
        .await;
    let client = Client::new(&config);
    get_bucket_region_with_client(client, bucket).await
}

/// Attempt to resolve the AWS region of an S3 bucket using a provided [Client].
pub async fn get_bucket_region_with_client(
    client: Client,
    bucket: impl ToString,
) -> Result<Region> {
    let bucket_name = bucket.to_string();

    match client
        .get_bucket_location()
        .bucket(&bucket_name)
        .send()
        .await
    {
        Ok(output) => {
            if let Some(constraint) = output.location_constraint() {
                Ok(Region::new(constraint.as_str().to_string()))
            } else {
                Ok(Region::new("us-east-1"))
            }
        }
        Err(e) => Err(DebianError::Io(std::io::Error::other(
            format!("S3 error: {:?}", e),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_config::Region;

    #[tokio::test]
    async fn test_s3_repository_client_creation() {
        let region = Region::new("us-east-1");
        let client = S3RepositoryClient::new_with_region(
            region.clone(),
            "test-bucket",
            Some("test-prefix"),
        ).await.unwrap();

        assert_eq!(client.bucket(), "test-bucket");
        assert_eq!(client.key_prefix(), Some("test-prefix"));
        assert_eq!(client.region(), &region);
    }

    #[tokio::test]
    async fn test_s3_path_to_key_with_prefix() {
        let region = Region::new("us-east-1");
        let client = S3RepositoryClient::new_with_region(
            region,
            "test-bucket",
            Some("my-repo"),
        ).await.unwrap();

        assert_eq!(client.path_to_key("dists/stable/Release"), "my-repo/dists/stable/Release");
        assert_eq!(client.path_to_key("/pool/main/package.deb"), "my-repo/pool/main/package.deb");
    }

    #[tokio::test]
    async fn test_s3_path_to_key_without_prefix() {
        let region = Region::new("us-east-1");
        let client = S3RepositoryClient::new_with_region(
            region,
            "test-bucket",
            None,
        ).await.unwrap();

        assert_eq!(client.path_to_key("dists/stable/Release"), "dists/stable/Release");
        assert_eq!(client.path_to_key("/pool/main/package.deb"), "pool/main/package.deb");
    }

    #[tokio::test]
    async fn test_s3_repository_client_url() {
        let region = Region::new("us-east-1");
        let client_with_prefix = S3RepositoryClient::new_with_region(
            region.clone(),
            "test-bucket",
            Some("my-repo"),
        ).await.unwrap();
        
        let url = client_with_prefix.url().unwrap();
        assert_eq!(url.to_string(), "s3://test-bucket/my-repo");

        let client_without_prefix = S3RepositoryClient::new_with_region(
            region,
            "test-bucket",
            None,
        ).await.unwrap();
        
        let url = client_without_prefix.url().unwrap();
        assert_eq!(url.to_string(), "s3://test-bucket");
    }

    #[tokio::test]
    async fn test_s3_release_client_path_to_key() {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = Client::new(&config);
        let release_client = S3ReleaseClient {
            client,
            bucket: "test-bucket".to_string(),
            key_prefix: Some("my-repo".to_string()),
            relative_path: "dists/stable".to_string(),
            release: ReleaseFile::from_reader(std::io::Cursor::new(
                "Origin: Test\nSuite: stable\n".as_bytes()
            )).unwrap(),
            fetch_compression: Compression::Gzip,
        };

        assert_eq!(
            release_client.path_to_key("main/binary-amd64/Packages"), 
            "my-repo/main/binary-amd64/Packages"
        );
    }
}
