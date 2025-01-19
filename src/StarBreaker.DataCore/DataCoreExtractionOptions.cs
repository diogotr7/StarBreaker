namespace StarBreaker.DataCore;

public class DataCoreExtractionOptions
{
    public required bool ShouldWriteMetadata { get; init; }
    public required bool ShouldWriteEmptyArrays { get; init; }
    public required bool ShouldWriteTypeNames { get; init; }
    public required bool ShouldWriteBaseTypeNames { get; init; }
    public required bool ShouldWriteDataTypes { get; init; }
    public required bool ShouldWriteNulls { get; init; }
}